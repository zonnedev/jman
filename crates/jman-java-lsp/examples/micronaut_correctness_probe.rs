use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use jman_java_lsp::{offset_to_position, read_message, write_message};
use serde_json::{Value, json};

fn main() {
    let arguments: Vec<String> = std::env::args().collect();
    if arguments.len() != 3 {
        eprintln!("usage: micronaut_correctness_probe LSP_BINARY MICRONAUT_CORE");
        std::process::exit(2);
    }
    let root = PathBuf::from(&arguments[2]);
    let bean_context = root.join("inject/src/main/java/io/micronaut/context/BeanContext.java");
    let default_context =
        root.join("inject/src/main/java/io/micronaut/context/DefaultBeanContext.java");
    let application_context =
        root.join("inject/src/main/java/io/micronaut/context/DefaultApplicationContext.java");
    let registry = root.join(
        "http-client/src/main/java/io/micronaut/http/client/netty/DefaultNettyHttpClientRegistry.java",
    );
    let coordinates =
        root.join("module-info/src/main/java/io/micronaut/module/info/MavenCoordinates.java");
    let runtime_module = root.join(
        "module-info-runtime/src/main/java/io/micronaut/module/info/runtime/MicronautRuntimeModule.java",
    );
    let runtime_test = root.join(
        "module-info-runtime/src/test/java/io/micronaut/module/info/runtime/MicronautModuleRuntimeInfoFactoryTest.java",
    );

    let mut child = Command::new(&arguments[1])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("start JMAN Java");
    let mut input = child.stdin.take().unwrap();
    let mut output = BufReader::new(child.stdout.take().unwrap());
    send(
        &mut input,
        json!({
            "jsonrpc":"2.0","id":1,"method":"initialize",
            "params":{
                "rootUri":format!("file://{}", root.display()),
                "initializationOptions":{"buildSystem":"gradle"}
            }
        }),
    );
    let initialized = receive(&mut output);
    assert!(
        initialized["result"]["capabilities"].is_object(),
        "{initialized}"
    );

    assert_navigation(
        &mut input,
        &mut output,
        &bean_context,
        "new DefaultBeanContext()",
        "DefaultBeanContext",
        &default_context,
        10,
    );
    assert_navigation(
        &mut input,
        &mut output,
        &default_context,
        "permits DefaultApplicationContext",
        "DefaultApplicationContext",
        &application_context,
        20,
    );
    assert_navigation(
        &mut input,
        &mut output,
        &registry,
        "import io.micronaut.context.BeanContext;",
        "BeanContext",
        &bean_context,
        30,
    );
    assert_navigation(
        &mut input,
        &mut output,
        &runtime_test,
        "new MavenCoordinates(\"io.micronaut\"",
        "MavenCoordinates",
        &coordinates,
        35,
    );
    assert_navigation(
        &mut input,
        &mut output,
        &runtime_test,
        "core.getChildren().size()",
        "getChildren",
        &runtime_module,
        36,
    );

    send(
        &mut input,
        json!({
            "jsonrpc":"2.0","id":40,"method":"workspace/executeCommand",
            "params":{"command":"jman.java.status","arguments":[]}
        }),
    );
    let status = receive(&mut output);
    assert_eq!(status["result"]["state"], "ready", "{status}");
    assert!(
        status["result"]["indexedDocuments"]
            .as_u64()
            .unwrap_or_default()
            > 2_000,
        "{status}"
    );

    send(
        &mut input,
        json!({"jsonrpc":"2.0","id":50,"method":"shutdown"}),
    );
    assert!(receive(&mut output)["result"].is_null());
    send(&mut input, json!({"jsonrpc":"2.0","method":"exit"}));
    drop(input);
    assert!(child.wait().expect("wait for JMAN Java").success());
    println!(
        "Micronaut Core main/test diagnostics, records, generics, and cross-module Java 25 navigation passed"
    );
}

fn assert_navigation(
    input: &mut impl std::io::Write,
    output: &mut impl std::io::BufRead,
    source_path: &Path,
    context: &str,
    token: &str,
    expected_path: &Path,
    id: u64,
) {
    let source = std::fs::read_to_string(source_path).expect("read Micronaut source");
    let uri = format!("file://{}", source_path.display());
    send(
        input,
        json!({
            "jsonrpc":"2.0","method":"textDocument/didOpen",
            "params":{"textDocument":{
                "uri":uri,"languageId":"java","version":1,"text":source
            }}
        }),
    );
    let diagnostics = receive(output);
    assert_eq!(diagnostics["method"], "textDocument/publishDiagnostics");
    assert!(
        diagnostics["params"]["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .all(|diagnostic| diagnostic["severity"] != 1),
        "{source_path:?}: {diagnostics}"
    );
    let context_offset = source
        .find(context)
        .unwrap_or_else(|| panic!("cannot find {context:?} in {}", source_path.display()));
    let token_offset = context_offset + context.find(token).expect("token in context");
    let cursor = token_offset + token.len() / 2;
    send(
        input,
        json!({
            "jsonrpc":"2.0","id":id,"method":"textDocument/definition",
            "params":{
                "textDocument":{"uri":uri},
                "position":offset_to_position(
                    &source,
                    source[..cursor].encode_utf16().count() as u64
                )
            }
        }),
    );
    let definition = receive(output);
    let expected_uri = format!("file://{}", expected_path.display());
    assert!(
        definition["result"].as_array().is_some_and(|locations| {
            locations
                .iter()
                .any(|location| location["uri"] == expected_uri)
        }),
        "expected {expected_uri}: {definition}"
    );
}

fn send(writer: &mut impl std::io::Write, message: Value) {
    write_message(writer, &message).expect("write LSP message");
}

fn receive(reader: &mut impl std::io::BufRead) -> Value {
    read_message(reader)
        .expect("read LSP message")
        .expect("LSP closed unexpectedly")
}
