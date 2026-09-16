use std::io::BufReader;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use jman_java_lsp::{offset_to_position, read_message, write_message};
use serde_json::{Value, json};

fn main() {
    let arguments: Vec<String> = std::env::args().collect();
    if arguments.len() != 4 {
        eprintln!("usage: subprocess_probe LSP_BINARY PROJECT_ROOT BUILD_SYSTEM");
        std::process::exit(2);
    }
    let binary = &arguments[1];
    let project_root = PathBuf::from(&arguments[2]);
    let build_system = &arguments[3];
    let owner =
        project_root.join("src/main/java/org/springframework/samples/petclinic/owner/Owner.java");
    let controller = project_root
        .join("src/main/java/org/springframework/samples/petclinic/owner/OwnerController.java");
    let owner_uri = format!("file://{}", owner.display());
    let controller_uri = format!("file://{}", controller.display());
    let owner_source = std::fs::read_to_string(&owner).expect("read Owner.java");
    let controller_source =
        std::fs::read_to_string(&controller).expect("read OwnerController.java");

    let mut child = Command::new(binary)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("start native LSP");
    let mut input = child.stdin.take().unwrap();
    let mut output = BufReader::new(child.stdout.take().unwrap());

    send(
        &mut input,
        json!({
            "jsonrpc":"2.0","id":1,"method":"initialize",
            "params":{
                "rootUri":format!("file://{}", project_root.display()),
                "initializationOptions":{"buildSystem":build_system}
            }
        }),
    );
    assert_eq!(receive(&mut output)["id"], 1);
    send(
        &mut input,
        json!({"jsonrpc":"2.0","method":"initialized","params":{}}),
    );
    open(&mut input, &owner_uri, &owner_source);
    assert_diagnostics(&receive(&mut output), &owner_uri);
    open(&mut input, &controller_uri, &controller_source);
    assert_diagnostics(&receive(&mut output), &controller_uri);

    let owner_use = controller_source
        .find("Owner owner")
        .expect("find Owner use");
    let position = offset_to_position(
        &controller_source,
        controller_source[..owner_use].encode_utf16().count() as u64,
    );
    send(
        &mut input,
        json!({
            "jsonrpc":"2.0","id":2,"method":"textDocument/definition",
            "params":{"textDocument":{"uri":controller_uri},"position":position}
        }),
    );
    assert_eq!(receive(&mut output)["result"][0]["uri"], owner_uri);

    send(
        &mut input,
        json!({
            "jsonrpc":"2.0","id":20,"method":"textDocument/completion",
            "params":{
                "textDocument":{"uri":controller_uri},
                "position":offset_to_position(
                    &controller_source,
                    controller_source[..owner_use + "Owner".len()].encode_utf16().count() as u64
                )
            }
        }),
    );
    assert!(
        receive(&mut output)["result"]["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["label"] == "Owner")
    );

    send(
        &mut input,
        json!({
            "jsonrpc":"2.0","id":21,"method":"textDocument/hover",
            "params":{"textDocument":{"uri":controller_uri},"position":position}
        }),
    );
    assert!(
        receive(&mut output)["result"]["contents"]["value"]
            .as_str()
            .unwrap()
            .contains("Owner")
    );

    send(
        &mut input,
        json!({
            "jsonrpc":"2.0","id":22,"method":"textDocument/semanticTokens/full",
            "params":{"textDocument":{"uri":controller_uri}}
        }),
    );
    assert!(
        !receive(&mut output)["result"]["data"]
            .as_array()
            .unwrap()
            .is_empty()
    );

    let import_source =
        "package org.springframework.samples.petclinic; class ImportProbe { HashMap value; }";
    let import_path =
        project_root.join("src/main/java/org/springframework/samples/petclinic/ImportProbe.java");
    let import_uri = format!("file://{}", import_path.display());
    open(&mut input, &import_uri, import_source);
    let import_diagnostics = receive(&mut output);
    let missing_type = import_diagnostics["params"]["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|diagnostic| {
            diagnostic["code"]
                .as_str()
                .is_some_and(|code| code.contains("cant.resolve"))
        })
        .expect("missing HashM diagnostic")
        .clone();
    let import_cursor = import_source.find("HashM").unwrap() + "HashM".len();
    send(
        &mut input,
        json!({
            "jsonrpc":"2.0","id":220,"method":"textDocument/completion",
            "params":{
                "textDocument":{"uri":import_uri},
                "position":offset_to_position(
                    import_source,
                    import_source[..import_cursor].encode_utf16().count() as u64
                )
            }
        }),
    );
    let import_completion = receive(&mut output);
    assert!(
        import_completion["result"]["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| {
                item["label"] == "HashMap"
                    && item["additionalTextEdits"][0]["newText"] == "import java.util.HashMap;\n"
            }),
        "{import_completion}"
    );
    send(
        &mut input,
        json!({
            "jsonrpc":"2.0","id":221,"method":"textDocument/codeAction",
            "params":{
                "textDocument":{"uri":import_uri},
                "range":missing_type["range"],
                "context":{"diagnostics":[missing_type]}
            }
        }),
    );
    let import_actions = receive(&mut output);
    assert!(
        import_actions["result"]
            .as_array()
            .unwrap()
            .iter()
            .any(|action| action["title"] == "Import java.util.HashMap"),
        "{import_actions}"
    );
    send(
        &mut input,
        json!({
            "jsonrpc":"2.0","method":"textDocument/didClose",
            "params":{"textDocument":{"uri":import_uri}}
        }),
    );
    assert_eq!(
        receive(&mut output)["method"],
        "textDocument/publishDiagnostics"
    );

    let external_source = "class ExternalProbe { java.util.List<String> values = java.util.Collections.emptyList(); void test() { values.remove(0); values.remove(\"x\"); } }";
    let external_path =
        project_root.join("src/main/java/org/springframework/samples/petclinic/ExternalProbe.java");
    let external_uri = format!("file://{}", external_path.display());
    open(&mut input, &external_uri, external_source);
    assert_diagnostics(&receive(&mut output), &external_uri);
    let empty_list = external_source.find("emptyList").unwrap();
    send(
        &mut input,
        json!({
            "jsonrpc":"2.0","id":23,"method":"textDocument/definition",
            "params":{
                "textDocument":{"uri":external_uri},
                "position":offset_to_position(
                    external_source,
                    external_source[..empty_list + 2].encode_utf16().count() as u64
                )
            }
        }),
    );
    let external_definition = receive(&mut output);
    let jdk_uri = external_definition["result"][0]["uri"]
        .as_str()
        .expect("JDK definition URI");
    assert!(jdk_uri.starts_with("file://"), "{external_definition}");
    let jdk_source = std::fs::read_to_string(jdk_uri.trim_start_matches("file://")).unwrap();
    assert!(jdk_source.contains("emptyList"));
    open(&mut input, jdk_uri, &jdk_source);
    let opened_external = receive(&mut output);
    assert_diagnostics(&opened_external, jdk_uri);
    assert!(
        opened_external["params"]["diagnostics"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let declaration = "List<T> emptyList()";
    let list_type = jdk_source.find(declaration).unwrap();
    send(
        &mut input,
        json!({
            "jsonrpc":"2.0","id":24,"method":"textDocument/hover",
            "params":{
                "textDocument":{"uri":jdk_uri},
                "position":offset_to_position(
                    &jdk_source,
                    jdk_source[..list_type + 2].encode_utf16().count() as u64
                )
            }
        }),
    );
    assert!(
        receive(&mut output)["result"]["contents"]["value"]
            .as_str()
            .unwrap()
            .contains("List")
    );
    send(
        &mut input,
        json!({
            "jsonrpc":"2.0","id":25,"method":"textDocument/semanticTokens/full",
            "params":{"textDocument":{"uri":jdk_uri}}
        }),
    );
    assert!(
        !receive(&mut output)["result"]["data"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    send(
        &mut input,
        json!({
            "jsonrpc":"2.0","id":26,"method":"textDocument/definition",
            "params":{
                "textDocument":{"uri":jdk_uri},
                "position":offset_to_position(
                    &jdk_source,
                    jdk_source[..list_type + 2].encode_utf16().count() as u64
                )
            }
        }),
    );
    let chained_definition = receive(&mut output);
    let list_uri = chained_definition["result"][0]["uri"]
        .as_str()
        .expect("chained JDK definition URI");
    let list_source = std::fs::read_to_string(list_uri.trim_start_matches("file://")).unwrap();
    assert!(list_source.contains("interface List"), "{list_uri}");
    send(
        &mut input,
        json!({
            "jsonrpc":"2.0","method":"textDocument/didClose",
            "params":{"textDocument":{"uri":jdk_uri}}
        }),
    );
    assert_eq!(
        receive(&mut output)["method"],
        "textDocument/publishDiagnostics"
    );
    let remove_int = external_source.find("remove(0)").unwrap();
    let remove_object = external_source.find("remove(\"x\")").unwrap();
    let mut overload_definitions = Vec::new();
    for (id, cursor) in [(28, remove_int), (29, remove_object)] {
        send(
            &mut input,
            json!({
                "jsonrpc":"2.0","id":id,"method":"textDocument/definition",
                "params":{
                    "textDocument":{"uri":external_uri},
                    "position":offset_to_position(
                        external_source,
                        external_source[..cursor + 2].encode_utf16().count() as u64
                    )
                }
            }),
        );
        overload_definitions.push(receive(&mut output)["result"][0].clone());
    }
    assert_eq!(
        overload_definitions[0]["uri"],
        overload_definitions[1]["uri"]
    );
    assert_ne!(
        overload_definitions[0]["range"],
        overload_definitions[1]["range"]
    );
    send(
        &mut input,
        json!({
            "jsonrpc":"2.0","method":"textDocument/didClose",
            "params":{"textDocument":{"uri":external_uri}}
        }),
    );
    assert_eq!(
        receive(&mut output)["method"],
        "textDocument/publishDiagnostics"
    );

    send(
        &mut input,
        json!({
            "jsonrpc":"2.0","id":3,"method":"workspace/symbol",
            "params":{"query":"Owner"}
        }),
    );
    assert!(
        receive(&mut output)["result"]
            .as_array()
            .unwrap()
            .iter()
            .any(|symbol| symbol["name"] == "Owner")
    );

    send(
        &mut input,
        json!({
            "jsonrpc":"2.0","id":30,"method":"textDocument/references",
            "params":{
                "textDocument":{"uri":controller_uri},
                "position":position,
                "context":{"includeDeclaration":true}
            }
        }),
    );
    let workspace_references = receive(&mut output);
    assert!(
        workspace_references["result"]
            .as_array()
            .unwrap()
            .iter()
            .any(|location| { location["uri"] != owner_uri && location["uri"] != controller_uri }),
        "{workspace_references}"
    );

    send(
        &mut input,
        json!({
            "jsonrpc":"2.0","id":4,"method":"textDocument/rename",
            "params":{
                "textDocument":{"uri":controller_uri},
                "position":position,
                "newName":"Customer"
            }
        }),
    );
    let rename = receive(&mut output);
    assert!(
        !rename["result"]["changes"][&owner_uri]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(
        !rename["result"]["changes"][&controller_uri]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(
        rename["result"]["changes"]
            .as_object()
            .unwrap()
            .keys()
            .any(|uri| uri != &owner_uri && uri != &controller_uri),
        "{rename}"
    );

    send(
        &mut input,
        json!({
            "jsonrpc":"2.0","method":"workspace/didChangeWatchedFiles",
            "params":{"changes":[{
                "uri":format!(
                    "file://{}",
                    project_root
                        .join(if build_system == "maven" {"pom.xml"} else {"build.gradle"})
                        .display()
                ),
                "type":2
            }]}
        }),
    );
    let mut reloaded = Vec::new();
    while reloaded.len() < 2 {
        let message = receive(&mut output);
        if message["method"] == "textDocument/publishDiagnostics"
            && let Some(uri) = message["params"]["uri"].as_str()
        {
            reloaded.push(uri.to_owned());
        }
    }
    reloaded.sort();
    let mut expected = vec![owner_uri.clone(), controller_uri.clone()];
    expected.sort();
    assert_eq!(reloaded, expected);

    send(
        &mut input,
        json!({"jsonrpc":"2.0","id":5,"method":"shutdown"}),
    );
    assert!(receive(&mut output)["result"].is_null());
    send(&mut input, json!({"jsonrpc":"2.0","method":"exit"}));
    drop(input);
    assert!(child.wait().expect("wait for LSP").success());
    println!(
        "native {build_system} LSP subprocess diagnostics/navigation/search/rename/reload passed"
    );
}

fn open(input: &mut impl std::io::Write, uri: &str, text: &str) {
    send(
        input,
        json!({
            "jsonrpc":"2.0","method":"textDocument/didOpen",
            "params":{"textDocument":{
                "uri":uri,"languageId":"java","version":1,"text":text
            }}
        }),
    );
}

fn assert_diagnostics(message: &Value, uri: &str) {
    assert_eq!(
        message["method"], "textDocument/publishDiagnostics",
        "{message}"
    );
    assert_eq!(message["params"]["uri"], uri);
    assert!(
        message["params"]["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .all(|diagnostic| diagnostic["severity"] != 1)
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
