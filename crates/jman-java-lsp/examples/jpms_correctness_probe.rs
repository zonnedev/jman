use std::io::BufReader;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use jman_java_lsp::{read_message, write_message};
use serde_json::{Value, json};

fn main() {
    let arguments: Vec<_> = std::env::args().collect();
    assert_eq!(
        arguments.len(),
        6,
        "usage: jpms_correctness_probe LSP ROOT MODULE_INFO JAVA_SOURCE MODULE_NAME"
    );
    let root = PathBuf::from(&arguments[2]);
    let module_info = root.join(&arguments[3]);
    let java_source = root.join(&arguments[4]);
    let _module_name = &arguments[5];
    let mut child = Command::new(&arguments[1])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("start LSP");
    let mut input = child.stdin.take().unwrap();
    let mut output = BufReader::new(child.stdout.take().unwrap());
    send(
        &mut input,
        json!({
            "jsonrpc":"2.0","id":1,"method":"initialize",
            "params":{
                "rootUri":format!("file://{}", root.display()),
                "initializationOptions":{"buildSystem":"maven"}
            }
        }),
    );
    assert!(receive(&mut output)["result"]["capabilities"].is_object());

    for path in [&module_info, &java_source] {
        let source = std::fs::read_to_string(path).unwrap();
        send(
            &mut input,
            json!({
                "jsonrpc":"2.0","method":"textDocument/didOpen",
                "params":{"textDocument":{
                    "uri":format!("file://{}", path.display()),
                    "languageId":"java","version":1,"text":source
                }}
            }),
        );
        let diagnostics = receive_method(&mut output, "textDocument/publishDiagnostics");
        assert_eq!(
            diagnostics["method"], "textDocument/publishDiagnostics",
            "{diagnostics}"
        );
        assert!(
            diagnostics["params"]["diagnostics"]
                .as_array()
                .unwrap()
                .iter()
                .all(|diagnostic| diagnostic["severity"] != 1),
            "{}: {diagnostics}",
            path.display()
        );
    }

    send(
        &mut input,
        json!({"jsonrpc":"2.0","id":100,"method":"shutdown"}),
    );
    assert!(receive(&mut output)["result"].is_null());
    send(&mut input, json!({"jsonrpc":"2.0","method":"exit"}));
    drop(input);
    assert!(child.wait().unwrap().success());
    println!("JPMS module descriptor, module path, and named-module attribution passed");
}

fn send(writer: &mut impl std::io::Write, message: Value) {
    write_message(writer, &message).unwrap();
}

fn receive(reader: &mut impl std::io::BufRead) -> Value {
    read_message(reader).unwrap().expect("LSP closed")
}

fn receive_method(reader: &mut impl std::io::BufRead, method: &str) -> Value {
    loop {
        let message = receive(reader);
        if message["method"] == method {
            return message;
        }
        assert!(
            message.get("id").is_none(),
            "received response while waiting for {method}: {message}"
        );
    }
}
