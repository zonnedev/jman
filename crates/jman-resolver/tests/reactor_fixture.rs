use jman_resolver::parse_pom;

#[test]
fn parses_the_spring_boot_reactor_contract() {
    let root = parse_pom(include_str!("fixtures/springboot-microservices/pom.xml"))
        .expect("root reactor POM");
    let child = parse_pom(include_str!(
        "fixtures/springboot-microservices/api-gateway/pom.xml"
    ))
    .expect("child module POM");

    assert_eq!(root.packaging, "pom");
    assert_eq!(root.modules, ["api-gateway"]);
    assert_eq!(root.dependency_management.len(), 1);
    assert_eq!(
        child.parent.as_ref().map(|parent| parent.artifact.as_str()),
        Some("springboot-microservices")
    );
    assert_eq!(child.dependencies.len(), 1);
}
