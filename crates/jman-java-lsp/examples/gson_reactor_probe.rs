use std::io::BufReader;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use jman_java_lsp::{offset_to_position, read_message, write_message};
use serde_json::{Value, json};

fn main() {
    let arguments: Vec<_> = std::env::args().collect();
    assert!(
        arguments.len() == 3 || arguments.len() == 4,
        "usage: gson_reactor_probe LSP GSON_ROOT [SOURCE]"
    );
    let root = PathBuf::from(&arguments[2]);
    let source_path = root.join(arguments.get(3).map(String::as_str).unwrap_or(
        "extras/src/main/java/com/google/gson/typeadapters/PostConstructAdapterFactory.java",
    ));
    let expected = root.join("gson/src/main/java/com/google/gson/Gson.java");
    let source = std::fs::read_to_string(&source_path).expect("Gson extras source");
    let uri = format!("file://{}", source_path.display());
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
    send(
        &mut input,
        json!({"jsonrpc":"2.0","method":"initialized","params":{}}),
    );
    send(
        &mut input,
        json!({
            "jsonrpc":"2.0","method":"textDocument/didOpen",
            "params":{"textDocument":{
                "uri":uri,"languageId":"java","version":1,"text":source
            }}
        }),
    );
    let diagnostics = receive_method(&mut output, "textDocument/publishDiagnostics");
    if arguments.len() == 4 {
        if let Some(sdk_symbol) = source.find("requireNonNull") {
            send(
                &mut input,
                json!({
                    "jsonrpc":"2.0","id":40,"method":"textDocument/definition",
                    "params":{
                        "textDocument":{"uri":uri},
                        "position":offset_to_position(
                            &source,
                            source[..sdk_symbol + 2].encode_utf16().count() as u64
                        )
                    }
                }),
            );
            let sdk_definition = receive(&mut output);
            let sdk_uri = sdk_definition["result"][0]["uri"]
                .as_str()
                .unwrap_or_else(|| panic!("SDK definition: {sdk_definition}"));
            let sdk_source =
                std::fs::read_to_string(sdk_uri.trim_start_matches("file://")).unwrap();
            assert!(sdk_source.contains("class Objects"), "{sdk_definition}");
            send(
                &mut input,
                json!({
                    "jsonrpc":"2.0","method":"textDocument/didOpen",
                    "params":{"textDocument":{
                        "uri":sdk_uri,"languageId":"java","version":1,"text":sdk_source
                    }}
                }),
            );
            receive_method(&mut output, "textDocument/publishDiagnostics");
            let nested = sdk_source
                .find("new NullPointerException")
                .expect("Objects creates NullPointerException")
                + "new ".len();
            send(
                &mut input,
                json!({
                    "jsonrpc":"2.0","id":41,"method":"textDocument/definition",
                    "params":{
                        "textDocument":{"uri":sdk_uri},
                        "position":offset_to_position(
                            &sdk_source,
                            sdk_source[..nested + 2].encode_utf16().count() as u64
                        )
                    }
                }),
            );
            let nested_definition = receive(&mut output);
            let nested_uri = nested_definition["result"][0]["uri"]
                .as_str()
                .expect("nested SDK definition");
            let nested_source =
                std::fs::read_to_string(nested_uri.trim_start_matches("file://")).unwrap();
            assert!(
                nested_source.contains("class NullPointerException"),
                "{nested_definition}"
            );
        }
        shutdown(&mut input, &mut output, child);
        println!("Gson modular SDK navigation chain passed");
        return;
    }
    assert!(
        diagnostics["params"]["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .all(|diagnostic| {
                let message = diagnostic["message"].as_str().unwrap_or_default();
                !message.contains("package com.google.gson")
                    && !message.contains("class Gson")
                    && !message.contains("class TypeAdapter")
                    && !message.contains("class TypeToken")
            }),
        "{diagnostics}"
    );
    let offset = source.find("Gson gson").expect("Gson field");
    send(
        &mut input,
        json!({
            "jsonrpc":"2.0","id":2,"method":"textDocument/definition",
            "params":{
                "textDocument":{"uri":uri},
                "position":offset_to_position(
                    &source,
                    source[..offset + 2].encode_utf16().count() as u64
                )
            }
        }),
    );
    let definition = receive(&mut output);
    assert_eq!(
        definition["result"][0]["uri"],
        format!("file://{}", expected.display()),
        "{definition}"
    );
    shutdown(&mut input, &mut output, child);
    println!("Gson Maven reactor attribution and navigation passed");
}

fn shutdown(
    input: &mut impl std::io::Write,
    output: &mut impl std::io::BufRead,
    mut child: std::process::Child,
) {
    send(input, json!({"jsonrpc":"2.0","id":3,"method":"shutdown"}));
    assert!(receive(output)["result"].is_null());
    send(input, json!({"jsonrpc":"2.0","method":"exit"}));
    assert!(child.wait().unwrap().success());
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
            "response while waiting for {method}: {message}"
        );
    }
}
