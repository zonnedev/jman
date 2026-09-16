use std::path::PathBuf;

use javac_frontend::Frontend;

fn main() {
    let arguments: Vec<_> = std::env::args().collect();
    assert_eq!(
        arguments.len(),
        6,
        "usage: module_semantic_probe SOURCE MODULE_INFO RELEASE MODULE_PATH SOURCE_PATH"
    );
    let source_path = PathBuf::from(&arguments[1]);
    let module_info = PathBuf::from(&arguments[2]);
    let release = arguments[3].parse().expect("release");
    let module_path: Vec<_> = std::env::split_paths(&arguments[4]).collect();
    let source_roots: Vec<_> = std::env::split_paths(&arguments[5]).collect();
    let source = std::fs::read_to_string(&source_path).expect("source");
    let frontend = Frontend::new().expect("frontend");
    let session = frontend
        .create_module_session(
            &[],
            &module_path,
            &source_roots,
            Some(&module_info),
            &[],
            release,
        )
        .expect("session");
    let result = session
        .analyze(source_path.file_name().unwrap().to_str().unwrap(), &source)
        .expect("analysis");
    for diagnostic in result
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.kind == "error")
    {
        eprintln!("{}: {}", diagnostic.code, diagnostic.message);
    }
    assert!(
        result
            .diagnostics
            .iter()
            .all(|diagnostic| diagnostic.kind != "error"),
        "semantic errors"
    );
    println!(
        "module semantic analysis passed with {} symbols",
        result.symbols.len()
    );
}
