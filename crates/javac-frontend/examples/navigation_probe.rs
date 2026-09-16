use std::path::{Path, PathBuf};
use std::time::Instant;

use javac_frontend::{Frontend, ProjectState};

fn main() {
    let arguments: Vec<String> = std::env::args().collect();
    if arguments.len() != 8 {
        eprintln!(
            "usage: navigation_probe DEFINITION_SOURCE REFERENCE_SOURCE RELEASE \
             CLASSPATH_FILE SOURCE_PATH QUALIFIED_NAME SEARCH_QUERY"
        );
        std::process::exit(2);
    }
    let definition_source = PathBuf::from(&arguments[1]);
    let reference_source = PathBuf::from(&arguments[2]);
    let release: u8 = arguments[3].parse().expect("release must be an integer");
    let classpath = std::fs::read_to_string(&arguments[4]).expect("read classpath");
    let classpath: Vec<PathBuf> = std::env::split_paths(classpath.trim()).collect();
    let source_path: Vec<PathBuf> = std::env::split_paths(&arguments[5]).collect();
    let qualified_name = &arguments[6];
    let query = &arguments[7];

    let frontend = Frontend::new().expect("create native frontend");
    let session = frontend
        .create_session(&classpath, &source_path, release)
        .expect("create project session");
    let started = Instant::now();
    let definition_result = analyze(&session, &definition_source);
    let reference_result = analyze(&session, &reference_source);
    let elapsed = started.elapsed();

    fail_on_errors(&definition_source, &definition_result);
    fail_on_errors(&reference_source, &reference_result);

    let definition_document = file_name(&definition_source);
    let reference_document = file_name(&reference_source);
    let mut project = ProjectState::default();
    project.update_document(definition_document, &definition_result);
    project.update_document(reference_document, &reference_result);

    let symbol_id = definition_result
        .symbols
        .iter()
        .find(|symbol| symbol.role == "declaration" && symbol.qualified_name == *qualified_name)
        .map(|symbol| symbol.symbol_id.as_str())
        .expect("canonical declaration identity");
    let definitions = project.definitions(symbol_id);
    let references = project.references(symbol_id);
    let search = project.workspace_symbols(query, 20);
    let old_name = qualified_name.rsplit('.').next().unwrap();
    let new_name = format!("{old_name}Renamed");
    let edits = project
        .prepare_rename(symbol_id, &new_name)
        .expect("prepare safe rename");
    assert!(
        definitions
            .iter()
            .any(|location| location.document == definition_document),
        "definition was not indexed in {definition_document}"
    );
    assert!(
        references
            .iter()
            .any(|location| location.document == reference_document),
        "cross-file reference was not indexed in {reference_document}"
    );
    assert!(
        search
            .iter()
            .any(|location| location.qualified_name == *qualified_name),
        "workspace search did not find {qualified_name}"
    );
    for edit in &edits {
        let path = if edit.document == definition_document {
            &definition_source
        } else if edit.document == reference_document {
            &reference_source
        } else {
            panic!("rename produced an edit for an unknown document");
        };
        let source = std::fs::read_to_string(path).expect("reread rename source");
        assert_eq!(
            &source[edit.start as usize..edit.end as usize],
            old_name,
            "rename range is not the exact identifier"
        );
    }
    println!(
        "{qualified_name}: {} definitions, {} references, {} search matches, {} rename edits, {:.2?}",
        definitions.len(),
        references.len(),
        search.len(),
        edits.len(),
        elapsed
    );
}

fn analyze(
    session: &javac_frontend::ProjectSession,
    path: &Path,
) -> javac_frontend::SemanticResult {
    let source = std::fs::read_to_string(path).expect("read Java source");
    session
        .analyze(file_name(path), &source)
        .expect("analyze Java source")
}

fn file_name(path: &Path) -> &str {
    path.file_name()
        .and_then(|name| name.to_str())
        .expect("source file name must be UTF-8")
}

fn fail_on_errors(path: &Path, result: &javac_frontend::SemanticResult) {
    let errors: Vec<_> = result
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.kind == "error")
        .collect();
    if !errors.is_empty() {
        for diagnostic in errors.iter().take(10) {
            eprintln!(
                "{}: {}: {}",
                path.display(),
                diagnostic.code,
                diagnostic.message
            );
        }
        std::process::exit(1);
    }
}
