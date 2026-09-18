use std::{collections::BTreeMap, path::PathBuf};

use roxmltree::{Document, Node};

use crate::{
    AnnotationProcessor, Dependency, Exclusion, Parent, RawPom, Relocation, Repository,
    ResolverError,
};

/// Parse the Maven model fields used by native resolution and import.
///
/// # Errors
///
/// Returns an error for malformed XML or missing required Maven fields.
pub fn parse_pom(xml: &str) -> Result<RawPom, ResolverError> {
    let document =
        Document::parse(xml).map_err(|error| ResolverError::InvalidPom(error.to_string()))?;
    let project = document.root_element();
    if project.tag_name().name() != "project" {
        return Err(ResolverError::InvalidPom(
            "root element must be <project>".to_owned(),
        ));
    }

    parse_project(project, true)
}

pub(crate) struct PrecomputedProject {
    pub directory: PathBuf,
    pub raw: RawPom,
}

pub(crate) fn parse_effective_workspace(
    xml: &str,
) -> Result<Vec<PrecomputedProject>, ResolverError> {
    let document =
        Document::parse(xml).map_err(|error| ResolverError::InvalidPom(error.to_string()))?;
    let root = document.root_element();
    let projects = match root.tag_name().name() {
        "project" => vec![root],
        "projects" => root
            .children()
            .filter(|node| node.is_element() && node.tag_name().name() == "project")
            .collect(),
        _ => {
            return Err(ResolverError::InvalidPom(
                "effective Maven model root must be <project> or <projects>".to_owned(),
            ));
        }
    };
    if projects.is_empty() {
        return Err(ResolverError::InvalidPom(
            "effective Maven model contains no projects".to_owned(),
        ));
    }
    projects
        .into_iter()
        .map(|project| {
            let directory = effective_project_directory(project)?;
            let mut raw = parse_project(project, false)?;
            apply_effective_compiler_configuration(project, &mut raw)?;
            Ok(PrecomputedProject { directory, raw })
        })
        .collect()
}

fn parse_project(project: Node<'_, '_>, evaluate_profiles: bool) -> Result<RawPom, ResolverError> {
    let parent = child(project, "parent").map(parse_parent).transpose()?;
    let artifact = child_text(project, "artifactId")
        .ok_or_else(|| ResolverError::InvalidPom("missing project artifactId".to_owned()))?;
    let mut properties = child(project, "properties")
        .map(parse_properties)
        .unwrap_or_default();
    let mut dependencies = child(project, "dependencies")
        .map(parse_dependencies)
        .transpose()?
        .unwrap_or_default();
    let mut dependency_management = child(project, "dependencyManagement")
        .and_then(|node| child(node, "dependencies"))
        .map(parse_dependencies)
        .transpose()?
        .unwrap_or_default();
    let mut repositories = child(project, "repositories")
        .map(parse_repositories)
        .transpose()?
        .unwrap_or_default();
    if evaluate_profiles {
        if let Some(profiles) = child(project, "profiles") {
            let candidates = profiles
                .children()
                .filter(|profile| profile.is_element() && profile.tag_name().name() == "profile")
                .collect::<Vec<_>>();
            let explicitly_active = candidates
                .iter()
                .copied()
                .filter(is_environment_active_profile)
                .collect::<Vec<_>>();
            let active = if explicitly_active.is_empty() {
                candidates
                    .into_iter()
                    .filter(is_active_by_default_profile)
                    .collect()
            } else {
                explicitly_active
            };
            for profile in active {
                if let Some(profile_properties) = child(profile, "properties") {
                    properties.extend(parse_properties(profile_properties));
                }
                if let Some(profile_dependencies) = child(profile, "dependencies") {
                    dependencies.extend(parse_dependencies(profile_dependencies)?);
                }
                if let Some(management) = child(profile, "dependencyManagement")
                    .and_then(|node| child(node, "dependencies"))
                {
                    dependency_management.extend(parse_dependencies(management)?);
                }
                if let Some(profile_repositories) = child(profile, "repositories") {
                    repositories.extend(parse_repositories(profile_repositories)?);
                }
            }
        }
    }
    let modules = child(project, "modules")
        .map(|modules| {
            modules
                .children()
                .filter(|module| module.is_element() && module.tag_name().name() == "module")
                .map(element_text)
                .filter(|module| !module.is_empty())
                .collect()
        })
        .unwrap_or_default();
    let relocation = child(project, "distributionManagement")
        .and_then(|management| child(management, "relocation"))
        .map(|relocation| Relocation {
            group: child_text(relocation, "groupId"),
            artifact: child_text(relocation, "artifactId"),
            version: child_text(relocation, "version"),
        });

    Ok(RawPom {
        model_version: child_text(project, "modelVersion"),
        parent,
        group: child_text(project, "groupId"),
        artifact,
        version: child_text(project, "version"),
        packaging: child_text(project, "packaging").unwrap_or_else(|| "jar".to_owned()),
        modules,
        properties,
        dependencies,
        dependency_management,
        repositories,
        annotation_processors: Vec::new(),
        compiler_args: Vec::new(),
        relocation,
    })
}

fn effective_project_directory(project: Node<'_, '_>) -> Result<PathBuf, ResolverError> {
    let build = child(project, "build");
    let directory = build
        .and_then(|build| child_text(build, "directory"))
        .and_then(|directory| PathBuf::from(directory).parent().map(PathBuf::from))
        .or_else(|| {
            build
                .and_then(|build| child_text(build, "sourceDirectory"))
                .and_then(|source| {
                    let source = PathBuf::from(source);
                    source
                        .ends_with("src/main/java")
                        .then(|| source.ancestors().nth(3).map(PathBuf::from))
                        .flatten()
                })
        })
        .ok_or_else(|| {
            ResolverError::InvalidPom(format!(
                "effective Maven project `{}` has no absolute build directory",
                child_text(project, "artifactId").unwrap_or_else(|| "unknown".to_owned())
            ))
        })?;
    if !directory.is_absolute() {
        return Err(ResolverError::InvalidPom(format!(
            "effective Maven project directory must be absolute: {}",
            directory.display()
        )));
    }
    Ok(directory)
}

fn apply_effective_compiler_configuration(
    project: Node<'_, '_>,
    raw: &mut RawPom,
) -> Result<(), ResolverError> {
    let Some(configuration) = child(project, "build")
        .and_then(|build| child(build, "plugins"))
        .and_then(|plugins| {
            plugins.children().filter(Node::is_element).find(|plugin| {
                plugin.tag_name().name() == "plugin"
                    && child_text(*plugin, "artifactId").as_deref() == Some("maven-compiler-plugin")
            })
        })
        .and_then(|plugin| child(plugin, "configuration"))
    else {
        return Ok(());
    };

    if let Some(release) = child_text(configuration, "release") {
        raw.properties
            .insert("maven.compiler.release".to_owned(), release);
    } else if let Some(source) = child_text(configuration, "source") {
        raw.properties
            .insert("maven.compiler.source".to_owned(), source);
    }
    if let Some(encoding) = child_text(configuration, "encoding") {
        raw.properties
            .insert("project.build.sourceEncoding".to_owned(), encoding);
    }
    if child_text(configuration, "parameters").as_deref() == Some("true") {
        raw.compiler_args.push("-parameters".to_owned());
    }
    if let Some(arguments) = child(configuration, "compilerArgs") {
        raw.compiler_args.extend(
            arguments
                .children()
                .filter(|argument| argument.is_element() && argument.tag_name().name() == "arg")
                .map(element_text)
                .filter(|argument| !argument.is_empty()),
        );
    }
    if let Some(processors) = child(configuration, "annotationProcessors") {
        let processors = processors
            .children()
            .filter(|processor| {
                processor.is_element() && processor.tag_name().name() == "annotationProcessor"
            })
            .map(element_text)
            .filter(|processor| !processor.is_empty())
            .collect::<Vec<_>>();
        if !processors.is_empty() {
            raw.compiler_args.push("-processor".to_owned());
            raw.compiler_args.push(processors.join(","));
        }
    }
    if let Some(paths) = child(configuration, "annotationProcessorPaths") {
        for path in paths
            .children()
            .filter(|path| path.is_element() && path.tag_name().name() == "path")
        {
            let group = required_text(path, "groupId", "annotation processor")?;
            let artifact = required_text(path, "artifactId", "annotation processor")?;
            let version = child_text(path, "version").or_else(|| {
                raw.dependency_management
                    .iter()
                    .find(|dependency| dependency.group == group && dependency.artifact == artifact)
                    .and_then(|dependency| dependency.version.clone())
            });
            raw.annotation_processors.push(AnnotationProcessor {
                dependency: Dependency {
                    group,
                    artifact,
                    version,
                    scope: "compile".to_owned(),
                    scope_explicit: false,
                    dependency_type: child_text(path, "type").unwrap_or_else(|| "jar".to_owned()),
                    type_explicit: child(path, "type").is_some(),
                    classifier: child_text(path, "classifier"),
                    optional: None,
                    exclusions: Vec::new(),
                },
            });
        }
    }
    Ok(())
}

/// Return Maven plugins declared by a POM without interpreting their behavior.
///
/// Both regular build plugins and plugin-management entries are reported,
/// including declarations inside profiles. Missing group IDs use Maven's
/// conventional `org.apache.maven.plugins` default.
///
/// # Errors
///
/// Returns an error when the XML is malformed.
pub fn plugin_coordinates(xml: &str) -> Result<Vec<String>, ResolverError> {
    let document =
        Document::parse(xml).map_err(|error| ResolverError::InvalidPom(error.to_string()))?;
    let project = document.root_element();
    let mut plugins = std::collections::BTreeSet::new();
    collect_build_plugins(project, &mut plugins);
    if let Some(profiles) = child(project, "profiles") {
        for profile in profiles
            .children()
            .filter(|node| node.is_element() && node.tag_name().name() == "profile")
        {
            collect_build_plugins(profile, &mut plugins);
        }
    }
    Ok(plugins.into_iter().collect())
}

fn collect_build_plugins(node: Node<'_, '_>, plugins: &mut std::collections::BTreeSet<String>) {
    let Some(build) = child(node, "build") else {
        return;
    };
    for container in [
        child(build, "plugins"),
        child(build, "pluginManagement").and_then(|management| child(management, "plugins")),
    ]
    .into_iter()
    .flatten()
    {
        for plugin in container
            .children()
            .filter(|node| node.is_element() && node.tag_name().name() == "plugin")
        {
            if let Some(artifact) = child_text(plugin, "artifactId") {
                let group = child_text(plugin, "groupId")
                    .unwrap_or_else(|| "org.apache.maven.plugins".to_owned());
                plugins.insert(format!("{group}:{artifact}"));
            }
        }
    }
}

fn parse_parent(node: Node<'_, '_>) -> Result<Parent, ResolverError> {
    Ok(Parent {
        group: required_text(node, "groupId", "parent")?,
        artifact: required_text(node, "artifactId", "parent")?,
        version: required_text(node, "version", "parent")?,
        relative_path: child(node, "relativePath").map(element_text),
    })
}

fn parse_properties(node: Node<'_, '_>) -> BTreeMap<String, String> {
    node.children()
        .filter(Node::is_element)
        .map(|property| {
            (
                property.tag_name().name().to_owned(),
                element_text(property),
            )
        })
        .collect()
}

fn parse_dependencies(node: Node<'_, '_>) -> Result<Vec<Dependency>, ResolverError> {
    node.children()
        .filter(|child| child.is_element() && child.tag_name().name() == "dependency")
        .map(parse_dependency)
        .collect()
}

fn parse_dependency(node: Node<'_, '_>) -> Result<Dependency, ResolverError> {
    let exclusions = child(node, "exclusions")
        .map(|items| {
            items
                .children()
                .filter(|item| item.is_element() && item.tag_name().name() == "exclusion")
                .map(|item| {
                    Ok(Exclusion {
                        group: required_text(item, "groupId", "exclusion")?,
                        artifact: required_text(item, "artifactId", "exclusion")?,
                    })
                })
                .collect()
        })
        .transpose()?
        .unwrap_or_default();

    let scope = child_text(node, "scope");
    let dependency_type = child_text(node, "type");
    let optional = child_text(node, "optional");
    Ok(Dependency {
        group: required_text(node, "groupId", "dependency")?,
        artifact: required_text(node, "artifactId", "dependency")?,
        version: child_text(node, "version"),
        scope_explicit: scope.is_some(),
        scope: scope.unwrap_or_else(|| "compile".to_owned()),
        type_explicit: dependency_type.is_some(),
        dependency_type: dependency_type.unwrap_or_else(|| "jar".to_owned()),
        classifier: child_text(node, "classifier"),
        optional: optional.map(|value| value == "true"),
        exclusions,
    })
}

fn parse_repositories(node: Node<'_, '_>) -> Result<Vec<Repository>, ResolverError> {
    node.children()
        .filter(|child| child.is_element() && child.tag_name().name() == "repository")
        .map(|repository| {
            Ok(Repository {
                id: required_text(repository, "id", "repository")?,
                url: required_text(repository, "url", "repository")?,
                releases_enabled: policy_enabled(repository, "releases"),
                snapshots_enabled: policy_enabled(repository, "snapshots"),
            })
        })
        .collect()
}

fn policy_enabled(repository: Node<'_, '_>, policy: &str) -> bool {
    child(repository, policy)
        .and_then(|node| child_text(node, "enabled"))
        .is_none_or(|value| value != "false")
}

fn child<'a>(node: Node<'a, 'a>, name: &str) -> Option<Node<'a, 'a>> {
    node.children()
        .find(|child| child.is_element() && child.tag_name().name() == name)
}

fn is_active_by_default_profile(profile: &Node<'_, '_>) -> bool {
    profile.is_element()
        && profile.tag_name().name() == "profile"
        && child(*profile, "activation")
            .and_then(|activation| child_text(activation, "activeByDefault"))
            .is_some_and(|active| active == "true")
}

fn is_environment_active_profile(profile: &Node<'_, '_>) -> bool {
    let Some(activation) = child(*profile, "activation") else {
        return false;
    };
    property_activation(activation) || os_activation(activation)
}

fn property_activation(activation: Node<'_, '_>) -> bool {
    let Some(property) = child(activation, "property") else {
        return false;
    };
    let Some(name) = child_text(property, "name") else {
        return false;
    };
    let expected = child_text(property, "value");
    let (negated_name, name) = name
        .strip_prefix('!')
        .map_or((false, name.as_str()), |name| (true, name));
    let value = std::env::var(name).ok().or_else(|| {
        std::env::var(
            name.chars()
                .map(|character| {
                    if character == '.' || character == '-' {
                        '_'
                    } else {
                        character.to_ascii_uppercase()
                    }
                })
                .collect::<String>(),
        )
        .ok()
    });
    if negated_name {
        return value.is_none();
    }
    match expected.as_deref() {
        None => value.is_some(),
        Some(expected) if expected.starts_with('!') => {
            value.as_deref() != expected.strip_prefix('!')
        }
        Some(expected) => value.as_deref() == Some(expected),
    }
}

fn os_activation(activation: Node<'_, '_>) -> bool {
    let Some(os) = child(activation, "os") else {
        return false;
    };
    let family_matches = child_text(os, "family").is_none_or(|family| match family.as_str() {
        "windows" => cfg!(windows),
        "unix" => cfg!(unix),
        "mac" => cfg!(target_os = "macos"),
        _ => false,
    });
    let name_matches =
        child_text(os, "name").is_none_or(|name| name.eq_ignore_ascii_case(std::env::consts::OS));
    let arch_matches =
        child_text(os, "arch").is_none_or(|arch| arch.eq_ignore_ascii_case(std::env::consts::ARCH));
    family_matches && name_matches && arch_matches
}

fn child_text(node: Node<'_, '_>, name: &str) -> Option<String> {
    child(node, name)
        .map(element_text)
        .filter(|text| !text.is_empty())
}

fn element_text(node: Node<'_, '_>) -> String {
    node.children()
        .filter(Node::is_text)
        .filter_map(|child| child.text())
        .collect::<String>()
        .trim()
        .to_owned()
}

fn required_text(node: Node<'_, '_>, name: &str, context: &str) -> Result<String, ResolverError> {
    child_text(node, name)
        .ok_or_else(|| ResolverError::InvalidPom(format!("missing {context} {name}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_namespaced_pom_without_translating_plugins() {
        let xml = r#"<project xmlns="http://maven.apache.org/POM/4.0.0">
              <modelVersion>4.0.0</modelVersion>
              <groupId>com.example</groupId><artifactId>demo</artifactId><version>1</version>
              <properties><java.version>21</java.version></properties>
              <modules><module>core</module><module> service </module></modules>
              <dependencies><dependency>
                <groupId>org.example</groupId><artifactId>library</artifactId>
                <exclusions><exclusion><groupId>x</groupId><artifactId>y</artifactId></exclusion></exclusions>
              </dependency></dependencies>
              <build><plugins><plugin>
                <artifactId>maven-compiler-plugin</artifactId>
                <configuration>
                  <annotationProcessorPaths><path>
                    <groupId>org.example</groupId><artifactId>processor</artifactId><version>2</version>
                  </path></annotationProcessorPaths>
                  <compilerArgs><arg>-Aexample=true</arg></compilerArgs>
                </configuration>
              </plugin></plugins>
              <pluginManagement><plugins><plugin>
                <groupId>com.example</groupId><artifactId>managed-plugin</artifactId>
              </plugin></plugins></pluginManagement></build>
            </project>"#;
        let pom = parse_pom(xml).expect("valid POM");

        assert_eq!(pom.properties["java.version"], "21");
        assert_eq!(pom.modules, ["core", "service"]);
        assert_eq!(pom.dependencies[0].exclusions[0].group, "x");
        assert!(pom.annotation_processors.is_empty());
        assert!(pom.compiler_args.is_empty());
        assert_eq!(
            plugin_coordinates(xml).expect("plugin coordinates"),
            [
                "com.example:managed-plugin",
                "org.apache.maven.plugins:maven-compiler-plugin"
            ]
        );
    }

    #[test]
    fn parses_maven_effective_workspace_into_precomputed_project_models() {
        let projects = parse_effective_workspace(
            r"<projects>
              <project><modelVersion>4.0.0</modelVersion>
                <groupId>com.example</groupId><artifactId>root</artifactId><version>1</version>
                <packaging>pom</packaging><modules><module>app</module></modules>
                <build><directory>/workspace/target</directory></build>
              </project>
              <project><modelVersion>4.0.0</modelVersion>
                <parent><groupId>com.example</groupId><artifactId>root</artifactId><version>1</version></parent>
                <groupId>com.example</groupId><artifactId>app</artifactId><version>1</version>
                <profiles><profile><activation><activeByDefault>true</activeByDefault></activation>
                  <dependencies><dependency><groupId>wrong</groupId><artifactId>duplicate</artifactId><version>1</version></dependency></dependencies>
                </profile></profiles>
                <dependencies><dependency><groupId>org.example</groupId><artifactId>library</artifactId><version>2</version></dependency></dependencies>
                <build><directory>/workspace/app/target</directory><plugins><plugin>
                  <artifactId>maven-compiler-plugin</artifactId><configuration>
                    <release>21</release><encoding>UTF-8</encoding><parameters>true</parameters>
                    <compilerArgs><arg>-Aexample=true</arg><arg>-Xlint:all</arg></compilerArgs>
                    <annotationProcessorPaths><path><groupId>org.example</groupId>
                      <artifactId>processor</artifactId><version>3</version></path></annotationProcessorPaths>
                  </configuration>
                </plugin></plugins></build>
              </project>
            </projects>",
        )
        .expect("Maven effective reactor");

        assert_eq!(projects.len(), 2);
        assert_eq!(projects[0].directory, PathBuf::from("/workspace"));
        assert_eq!(projects[1].directory, PathBuf::from("/workspace/app"));
        assert_eq!(projects[1].raw.dependencies.len(), 1);
        assert_eq!(projects[1].raw.dependencies[0].artifact, "library");
        assert_eq!(projects[1].raw.properties["maven.compiler.release"], "21");
        assert_eq!(
            projects[1].raw.compiler_args,
            ["-parameters", "-Aexample=true", "-Xlint:all"]
        );
        assert_eq!(
            projects[1].raw.annotation_processors[0].dependency.artifact,
            "processor"
        );
    }

    #[test]
    fn parses_element_text_interrupted_by_xml_comments() {
        let pom = parse_pom(
            r"<project>
              <modelVersion>4.0.0</modelVersion>
              <groupId>org.checkerframework</groupId>
              <artifactId>checker-compat-qual</artifactId>
              <version><!-- release-version -->2.5.3<!-- /release-version --></version>
              <properties><combined>before<!-- separator -->after</combined></properties>
              <modules><module><!-- module -->core</module></modules>
            </project>",
        )
        .expect("comments inside text elements are valid Maven XML");

        assert_eq!(pom.version.as_deref(), Some("2.5.3"));
        assert_eq!(pom.properties["combined"], "beforeafter");
        assert_eq!(pom.modules, ["core"]);
    }

    #[test]
    fn merges_active_by_default_profile_model_fields() {
        let pom = parse_pom(
            r"<project>
              <modelVersion>4.0.0</modelVersion>
              <groupId>com.example</groupId><artifactId>client</artifactId><version>1</version>
              <profiles><profile><id>default</id>
                <activation><activeByDefault>true</activeByDefault></activation>
                <properties><profile.version>2</profile.version></properties>
                <dependencies><dependency><groupId>com.example</groupId>
                  <artifactId>core</artifactId><version>${profile.version}</version>
                </dependency></dependencies>
              </profile></profiles>
            </project>",
        )
        .expect("active-by-default profiles are part of Maven's effective model");

        assert_eq!(pom.properties["profile.version"], "2");
        assert_eq!(pom.dependencies[0].artifact, "core");
    }

    #[cfg(unix)]
    #[test]
    fn environment_profile_suppresses_active_by_default_profile() {
        let pom = parse_pom(
            r"<project>
              <modelVersion>4.0.0</modelVersion>
              <groupId>com.example</groupId><artifactId>client</artifactId><version>1</version>
              <profiles>
                <profile><id>default</id>
                  <activation><activeByDefault>true</activeByDefault></activation>
                  <dependencies><dependency><groupId>com.example</groupId>
                    <artifactId>default-dependency</artifactId><version>1</version>
                  </dependency></dependencies>
                </profile>
                <profile><id>unix</id><activation><os><family>unix</family></os></activation>
                  <dependencies><dependency><groupId>com.example</groupId>
                    <artifactId>unix-dependency</artifactId><version>1</version>
                  </dependency></dependencies>
                </profile>
              </profiles>
            </project>",
        )
        .expect("environment profile");

        assert_eq!(pom.dependencies.len(), 1);
        assert_eq!(pom.dependencies[0].artifact, "unix-dependency");
    }

    #[test]
    fn rejects_non_project_xml() {
        assert!(matches!(
            parse_pom("<settings/>"),
            Err(ResolverError::InvalidPom(message)) if message.contains("project")
        ));
    }
}
