#![recursion_limit = "256"]

#[cfg(feature = "native-ffi")]
mod async_native_backend;
mod cache;
mod documents;
mod metrics;
#[cfg(feature = "native-ffi")]
mod native_backend;
mod processor_worker;
mod project;
mod protocol;
mod server;
#[cfg(feature = "native-ffi")]
mod workspace_backend;

#[cfg(feature = "native-ffi")]
pub use async_native_backend::AsyncNativeBackend;
pub use documents::{Document, Documents, offset_to_position, position_to_byte};
#[cfg(feature = "native-ffi")]
pub use native_backend::NativeBackend;
pub use processor_worker::{ProcessorRequest, ProcessorResult, ProcessorWorker};
pub use project::{
    BuildTool, CompileModel, LoadedProject, ModelResolutionError, detect_build_tools,
    file_uri_to_path, load_project, parse_models, select_compile_model,
};
pub use protocol::{ProtocolError, read_message, write_message};
pub use server::{AnalysisBackend, CacheStatus, Dispatch, JpmsCatalog, Server, SourceMetadata};
#[cfg(feature = "native-ffi")]
pub use workspace_backend::WorkspaceBackend;

use std::io::{BufRead, Write};

/// Serve LSP messages from `reader` to `writer`.
///
/// Keeping the transport loop in the library lets applications such as `jman`
/// embed the language server without spawning a second executable.
pub fn run_transport(
    mut server: Server<'_>,
    reader: &mut impl BufRead,
    writer: &mut impl Write,
) -> i32 {
    loop {
        let message = match read_message(reader) {
            Ok(Some(message)) => message,
            Ok(None) => return 0,
            Err(error) => {
                eprintln!("jman-java-lsp protocol error: {error}");
                return 1;
            }
        };
        match server.dispatch(message) {
            Dispatch::Reply(reply) => {
                if let Err(error) = write_message(writer, &reply) {
                    eprintln!("jman-java-lsp write error: {error}");
                    return 1;
                }
            }
            Dispatch::Batch(messages) => {
                for message in messages {
                    if let Err(error) = write_message(writer, &message) {
                        eprintln!("jman-java-lsp write error: {error}");
                        return 1;
                    }
                }
            }
            Dispatch::Notification => {}
            Dispatch::Exit(code) => return code,
        }
    }
}

/// Run the Java language server over the process standard streams.
pub fn run_stdio() -> i32 {
    use std::io::{self, BufReader};

    let input = io::stdin();
    let output = io::stdout();
    #[cfg(feature = "native-ffi")]
    let server = Server::with_backend(WorkspaceBackend::new());
    #[cfg(not(feature = "native-ffi"))]
    let server = Server::default();

    run_transport(
        server,
        &mut BufReader::new(input.lock()),
        &mut output.lock(),
    )
}

#[cfg(test)]
mod transport_tests {
    use std::io::Cursor;

    use super::{Server, run_transport};

    fn frame(message: &str) -> Vec<u8> {
        format!("Content-Length: {}\r\n\r\n{message}", message.len()).into_bytes()
    }

    #[test]
    fn clean_eof_is_successful() {
        let mut input = Cursor::new(Vec::new());
        let mut output = Vec::new();

        assert_eq!(run_transport(Server::default(), &mut input, &mut output), 0);
        assert!(output.is_empty());
    }

    #[test]
    fn malformed_protocol_input_fails_without_writing_protocol_garbage() {
        let mut input = Cursor::new(b"not an lsp header\r\n\r\n".to_vec());
        let mut output = Vec::new();

        assert_eq!(run_transport(Server::default(), &mut input, &mut output), 1);
        assert!(output.is_empty());
    }

    #[test]
    fn initialize_and_exit_are_served_in_process() {
        let initialize =
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"capabilities":{}}}"#;
        let shutdown = r#"{"jsonrpc":"2.0","id":2,"method":"shutdown","params":null}"#;
        let exit = r#"{"jsonrpc":"2.0","method":"exit","params":null}"#;
        let mut bytes = frame(initialize);
        bytes.extend(frame(shutdown));
        bytes.extend(frame(exit));
        let mut input = Cursor::new(bytes);
        let mut output = Vec::new();

        assert_eq!(run_transport(Server::default(), &mut input, &mut output), 0);
        let response = String::from_utf8(output).expect("UTF-8 LSP response");
        assert!(response.starts_with("Content-Length: "));
        assert!(response.contains(r#""jsonrpc":"2.0""#));
        assert!(response.contains(r#""id":1"#));
        assert!(response.contains(r#""capabilities""#));
    }
}
