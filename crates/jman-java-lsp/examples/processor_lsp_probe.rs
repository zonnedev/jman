use std::io::BufReader;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use jman_java_lsp::{offset_to_position, read_message, write_message};
use serde_json::{Value, json};

fn main() {
    let arguments: Vec<String> = std::env::args().collect();
    if arguments.len() != 3 {
        eprintln!("usage: processor_lsp_probe LSP_BINARY PROJECT_ROOT");
        std::process::exit(2);
    }
    let root = PathBuf::from(&arguments[2]);
    let source = root.join(
        "app/src/processorTest/java/io/github/zonnedev/jman/tests/fixture/ProcessorTestProbe.java",
    );
    let generated = root.join(
        "app/build/generated/sources/annotationProcessor/java/processorTest/io/github/zonnedev/jman/tests/fixture/GeneratedGreeting.java",
    );
    let uri = format!("file://{}", source.display());
    let text = std::fs::read_to_string(&source).expect("read Application.java");
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
                "initializationOptions":{"buildSystem":"gradle"}
            }
        }),
    );
    let initialized = receive(&mut output);
    assert!(
        initialized["result"]["capabilities"].is_object(),
        "{initialized}"
    );
    send(
        &mut input,
        json!({
            "jsonrpc":"2.0","method":"textDocument/didOpen",
            "params":{"textDocument":{
                "uri":uri,"languageId":"java","version":1,"text":text
            }}
        }),
    );
    let diagnostics = receive(&mut output);
    assert!(
        diagnostics["params"]["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .all(|diagnostic| diagnostic["severity"] != 1),
        "{diagnostics}"
    );
    assert!(generated.is_file(), "{}", generated.display());
    let generated_token = text.find("GeneratedGreeting").unwrap();
    send(
        &mut input,
        json!({
            "jsonrpc":"2.0","id":10,"method":"textDocument/definition",
            "params":{
                "textDocument":{"uri":uri},
                "position":offset_to_position(
                    &text,
                    text[..generated_token + 4].encode_utf16().count() as u64
                )
            }
        }),
    );
    let definition = receive(&mut output);
    let generated_uri = format!("file://{}", generated.display());
    assert!(
        definition["result"].as_array().is_some_and(|locations| {
            locations
                .iter()
                .any(|location| location["uri"] == generated_uri)
        }),
        "{definition}"
    );
    for (id, query) in [(2, "GeneratedGreeting"), (3, "PersonMapperImpl")] {
        send(
            &mut input,
            json!({
                "jsonrpc":"2.0","id":id,"method":"workspace/symbol",
                "params":{"query":query}
            }),
        );
        let response = receive(&mut output);
        assert!(
            response["result"]
                .as_array()
                .unwrap()
                .iter()
                .any(|symbol| symbol["name"] == query),
            "{response}"
        );
    }
    verify_lombok_semantics(&mut input, &mut output, &root);
    let disk_source = std::fs::read_to_string(&source).unwrap();
    let without_annotation = disk_source
        .replace("@GenerateGreeting\n", "")
        .replace("return GeneratedGreeting.message();", "return \"plain\";");
    std::fs::write(&source, &without_annotation).unwrap();
    send(
        &mut input,
        json!({
            "jsonrpc":"2.0","method":"textDocument/didSave",
            "params":{"textDocument":{"uri":uri}}
        }),
    );
    let regenerated = receive_diagnostics_for(&mut output, &uri);
    assert_eq!(regenerated["method"], "textDocument/publishDiagnostics");
    assert!(
        !generated.exists(),
        "stale generated source survived save: {}",
        generated.display()
    );
    std::fs::write(&source, disk_source).unwrap();
    send(
        &mut input,
        json!({
            "jsonrpc":"2.0","method":"textDocument/didSave",
            "params":{"textDocument":{"uri":uri}}
        }),
    );
    let regenerated = receive_diagnostics_for(&mut output, &uri);
    assert_eq!(regenerated["method"], "textDocument/publishDiagnostics");
    assert!(
        generated.is_file(),
        "generated source was not restored: {}",
        generated.display()
    );

    send(
        &mut input,
        json!({"jsonrpc":"2.0","id":4,"method":"shutdown"}),
    );
    let _ = receive(&mut output);
    send(&mut input, json!({"jsonrpc":"2.0","method":"exit"}));
    drop(input);
    assert!(child.wait().unwrap().success());
    println!(
        "LSP custom-source-set processor generation, stale removal, refresh, and generated-symbol indexing passed"
    );
}

fn verify_lombok_semantics(
    input: &mut impl std::io::Write,
    output: &mut impl std::io::BufRead,
    root: &std::path::Path,
) {
    let source = root.join("app/src/main/java/io/github/zonnedev/jman/tests/fixture/Person.java");
    let uri = format!("file://{}", source.display());
    let text = std::fs::read_to_string(&source).expect("read Person.java");
    send(
        input,
        json!({
            "jsonrpc":"2.0","method":"textDocument/didOpen",
            "params":{"textDocument":{
                "uri":uri,"languageId":"java","version":1,"text":text
            }}
        }),
    );
    let diagnostics = receive(output);
    assert!(
        diagnostics["params"]["diagnostics"]
            .as_array()
            .is_some_and(|items| items.iter().all(|item| item["severity"] != 1)),
        "Lombok-processed source has errors: {diagnostics}"
    );

    let getter = text.find("getName").expect("getter call");
    send(
        input,
        json!({
            "jsonrpc":"2.0","id":20,"method":"textDocument/hover",
            "params":{
                "textDocument":{"uri":uri},
                "position":offset_to_position(
                    &text,
                    text[..getter + 3].encode_utf16().count() as u64
                )
            }
        }),
    );
    let hover = receive(output);
    assert!(
        hover["result"]["contents"]["value"]
            .as_str()
            .is_some_and(|detail| detail.contains("String")),
        "Lombok-generated getter has no semantic hover: {hover}"
    );

    let log = text.find("log.debug").expect("log field");
    send(
        input,
        json!({
            "jsonrpc":"2.0","id":21,"method":"textDocument/hover",
            "params":{
                "textDocument":{"uri":uri},
                "position":offset_to_position(
                    &text,
                    text[..log + 1].encode_utf16().count() as u64
                )
            }
        }),
    );
    let hover = receive(output);
    assert!(
        hover["result"]["contents"]["value"]
            .as_str()
            .is_some_and(|detail| detail.contains("org.slf4j.Logger")),
        "Lombok-generated log field has no semantic hover: {hover}"
    );

    send(
        input,
        json!({
            "jsonrpc":"2.0","id":22,"method":"textDocument/completion",
            "params":{
                "textDocument":{"uri":uri},
                "position":offset_to_position(
                    &text,
                    text[..getter + "getN".len()].encode_utf16().count() as u64
                )
            }
        }),
    );
    let completion = receive(output);
    assert!(
        completion["result"]["items"]
            .as_array()
            .is_some_and(|items| items.iter().any(|item| item["label"] == "getName")),
        "Lombok-generated getter has no completion: {completion}"
    );

    send(
        input,
        json!({
            "jsonrpc":"2.0","id":23,"method":"textDocument/definition",
            "params":{
                "textDocument":{"uri":uri},
                "position":offset_to_position(
                    &text,
                    text[..getter + 3].encode_utf16().count() as u64
                )
            }
        }),
    );
    let definition = receive(output);
    assert!(
        definition["result"]
            .as_array()
            .is_some_and(|locations| locations.iter().any(|location| location["uri"] == uri)),
        "Lombok-generated getter did not navigate to its local source: {definition}"
    );

    let broken = text.replacen(
        "public String describe() {",
        "public String describe() {\n    MissingType mustRemainAnError = null;",
        1,
    );
    send(
        input,
        json!({
            "jsonrpc":"2.0","method":"textDocument/didChange",
            "params":{
                "textDocument":{"uri":uri,"version":2},
                "contentChanges":[{"text":broken}]
            }
        }),
    );
    let diagnostics = receive_diagnostics_for(output, &uri);
    assert!(
        diagnostics["params"]["diagnostics"]
            .as_array()
            .is_some_and(|items| items.iter().any(|item| {
                item["message"]
                    .as_str()
                    .is_some_and(|message| message.contains("MissingType"))
            })),
        "real compiler errors must not be hidden by processor support: {diagnostics}"
    );

    let changed = text.replacen(
        "public String describe() {",
        "public String describe() {\n    log.debug(\"unsaved {}\", this.getName());",
        1,
    );
    send(
        input,
        json!({
            "jsonrpc":"2.0","method":"textDocument/didChange",
            "params":{
                "textDocument":{"uri":uri,"version":3},
                "contentChanges":[{"text":changed}]
            }
        }),
    );
    let diagnostics = receive_diagnostics_for(output, &uri);
    assert!(
        diagnostics["params"]["diagnostics"]
            .as_array()
            .is_some_and(|items| items.iter().all(|item| item["severity"] != 1)),
        "unsaved Lombok-aware overlay has errors: {diagnostics}"
    );
}

fn send(writer: &mut impl std::io::Write, message: Value) {
    write_message(writer, &message).expect("write LSP message");
}

fn receive(reader: &mut impl std::io::BufRead) -> Value {
    read_message(reader)
        .expect("read LSP message")
        .expect("server closed")
}

fn receive_diagnostics_for(reader: &mut impl std::io::BufRead, uri: &str) -> Value {
    loop {
        let message = receive(reader);
        if message["method"] == "textDocument/publishDiagnostics" && message["params"]["uri"] == uri
        {
            return message;
        }
    }
}
