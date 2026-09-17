use std::path::PathBuf;

use jman_java_lsp::{ProcessorRequest, ProcessorWorker};

fn main() {
    let arguments: Vec<String> = std::env::args().collect();
    if arguments.len() != 8 {
        eprintln!(
            "usage: processor_worker_probe JAVA WORKER_CLASSES SOURCE PROCESSOR_JAR GENERATED CLASSES REQUEST"
        );
        std::process::exit(2);
    }
    let source = PathBuf::from(&arguments[3]);
    let processor = PathBuf::from(&arguments[4]);
    let request = ProcessorRequest {
        java_executable: PathBuf::from(&arguments[1]),
        sources: vec![source.clone()],
        source_path: source.parent().map(PathBuf::from).into_iter().collect(),
        classpath: vec![processor.clone()],
        processor_path: vec![processor],
        processor_options: vec!["-Agreeting.mode=strict".to_owned()],
        release: 25,
        generated_directory: PathBuf::from(&arguments[5]),
        classes_directory: PathBuf::from(&arguments[6]),
    };
    let mut worker =
        ProcessorWorker::start(&PathBuf::from(&arguments[1]), &PathBuf::from(&arguments[2]))
            .expect("start processor worker");
    let request_file = PathBuf::from(&arguments[7]);

    let first = worker
        .process("fixture", &request, &request_file)
        .expect("first processor run");
    assert!(!first.cache_hit);
    assert_eq!(first.generated_sources, 1);
    let generated_class = request
        .classes_directory
        .join("io/github/zonnedev/jman/tests/fixture/GeneratedGreeting.class");
    assert!(
        generated_class.is_file(),
        "processor output must be compiled for Lombok-style binary augmentation"
    );
    let cached = worker
        .process("fixture", &request, &request_file)
        .expect("cached processor run");
    assert!(cached.cache_hit);

    let original = std::fs::read_to_string(&source).expect("read source");
    std::fs::write(&source, format!("{original}\n")).expect("touch source content");
    let invalidated = worker
        .process("fixture", &request, &request_file)
        .expect("invalidated processor run");
    assert!(!invalidated.cache_hit);
    assert_eq!(invalidated.generated_sources, 1);
    let generated = request
        .generated_directory
        .join("io/github/zonnedev/jman/tests/fixture/GeneratedGreeting.java");
    std::fs::write(
        &source,
        "package io.github.zonnedev.jman.tests.fixture; public final class Application {}",
    )
    .expect("remove processor annotation");
    let cleaned = worker
        .process("fixture", &request, &request_file)
        .expect("successful processor run without generated output");
    assert_eq!(cleaned.generated_sources, 0);
    assert!(
        !generated.exists(),
        "stale generated source must be removed after a successful run"
    );
    std::fs::write(&source, &original).expect("restore annotated source");
    worker
        .process("fixture", &request, &request_file)
        .expect("regenerate last good source");
    let last_good = std::fs::read_to_string(&generated).expect("read regenerated source");
    std::fs::write(&source, "this is not Java").expect("write invalid source");
    assert!(worker.process("fixture", &request, &request_file).is_err());
    assert_eq!(
        std::fs::read_to_string(&generated).expect("preserve generated source"),
        last_good
    );
    std::fs::write(&source, original).expect("restore source");
    println!("processor worker generation/cache/invalidation/rollback passed");
}
