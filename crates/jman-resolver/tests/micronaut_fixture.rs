use jman_resolver::{parse_pom, plugin_coordinates};

#[test]
fn parses_the_micronaut_acceptance_project_contract() {
    let pom = parse_pom(include_str!("fixtures/micronaut-demo/pom.xml")).expect("Micronaut POM");

    assert_eq!(
        pom.parent.as_ref().map(|parent| parent.artifact.as_str()),
        Some("micronaut-parent")
    );
    assert_eq!(pom.dependencies.len(), 11);
    assert!(pom.annotation_processors.is_empty());
    assert!(pom.compiler_args.is_empty());
    assert_eq!(pom.properties["jdk.version"], "25");
    assert_eq!(
        plugin_coordinates(include_str!("fixtures/micronaut-demo/pom.xml"))
            .expect("plugin coordinates"),
        ["org.apache.maven.plugins:maven-compiler-plugin"]
    );
}
