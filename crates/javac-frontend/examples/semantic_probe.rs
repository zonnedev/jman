use std::path::PathBuf;
use std::time::Instant;

use javac_frontend::{DependencyGraph, Frontend};

fn main() {
    let arguments: Vec<String> = std::env::args().collect();
    if arguments.len() != 6 {
        eprintln!(
            "usage: semantic_probe SOURCE RELEASE CLASSPATH_FILE SOURCE_ROOT EXPECTED_SYMBOL"
        );
        std::process::exit(2);
    }
    let source_path = PathBuf::from(&arguments[1]);
    let release: u8 = arguments[2].parse().expect("release must be an integer");
    let classpath = std::fs::read_to_string(&arguments[3]).expect("read classpath");
    let classpath: Vec<PathBuf> = std::env::split_paths(classpath.trim()).collect();
    let source = std::fs::read_to_string(&source_path).expect("read Java source");
    let source_roots: Vec<PathBuf> = std::env::split_paths(&arguments[4]).collect();
    let file_name = source_path
        .file_name()
        .and_then(|name| name.to_str())
        .expect("source file name must be UTF-8");

    let frontend = Frontend::new().expect("create native frontend");
    let session = frontend
        .create_session(&classpath, &source_roots, release)
        .expect("create project session");
    let cold_started = Instant::now();
    let result = session
        .analyze(file_name, &source)
        .expect("analyze project source");
    let cold_elapsed = cold_started.elapsed();
    let mut dependencies = DependencyGraph::default();
    let affected = dependencies.update_document(file_name, &result);
    for document in affected {
        session
            .invalidate(&document)
            .expect("invalidate affected document");
    }
    let invalidated_started = Instant::now();
    let invalidated_result = session
        .analyze(file_name, &source)
        .expect("reanalyze invalidated project source");
    let invalidated_elapsed = invalidated_started.elapsed();
    let cached_started = Instant::now();
    let cached_result = session
        .analyze(file_name, &source)
        .expect("reuse cached project source");
    let cached_elapsed = cached_started.elapsed();
    let expected = &arguments[5];
    if !result
        .symbols
        .iter()
        .any(|symbol| &symbol.qualified_name == expected)
    {
        eprintln!(
            "missing symbol {expected}; attributed {} symbols and {} diagnostics",
            result.symbols.len(),
            result.diagnostics.len()
        );
        std::process::exit(1);
    }
    for diagnostic in result
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.kind == "error")
        .take(10)
    {
        eprintln!("{}: {}", diagnostic.code, diagnostic.message);
    }
    if result
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.kind == "error")
    {
        std::process::exit(1);
    }
    if result != invalidated_result || result != cached_result {
        eprintln!("session result changed across invalidation or caching");
        std::process::exit(1);
    }
    println!(
        "{}: {} symbols, {} diagnostics, cold {:.2?}, invalidated {:.2?}, cached {:.2?}",
        source_path.display(),
        result.symbols.len(),
        result.diagnostics.len(),
        cold_elapsed,
        invalidated_elapsed,
        cached_elapsed
    );
}
