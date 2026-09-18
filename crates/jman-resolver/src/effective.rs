use std::{
    collections::{BTreeMap, HashMap},
    future::Future,
    pin::Pin,
    sync::Mutex,
};

use futures::{stream, StreamExt, TryStreamExt};

use crate::{
    parse_pom, AnnotationProcessor, Coordinate, Dependency, EffectivePom, RawPom, Repository,
    ResolverError,
};

pub trait PomSource: Send + Sync {
    fn load_pom(
        &self,
        coordinate: &Coordinate,
    ) -> Pin<Box<dyn Future<Output = Result<String, ResolverError>> + Send + '_>>;
}

pub struct EffectiveModelBuilder<S> {
    source: S,
    cache: Mutex<HashMap<String, EffectivePom>>,
}

impl<S: PomSource> EffectiveModelBuilder<S> {
    #[must_use]
    pub fn new(source: S) -> Self {
        Self {
            source,
            cache: Mutex::new(HashMap::new()),
        }
    }

    /// Construct an effective model for a local project POM.
    ///
    /// # Errors
    ///
    /// Returns an error when parsing, inheritance, imports, or interpolation fail.
    pub async fn build_local(&self, xml: &str) -> Result<EffectivePom, ResolverError> {
        let raw = parse_pom(xml)?;
        self.build_raw(raw, Vec::new()).await
    }

    pub(crate) async fn build_precomputed(
        &self,
        mut raw: RawPom,
    ) -> Result<EffectivePom, ResolverError> {
        let parent = raw
            .parent
            .take()
            .map(|parent| Coordinate::pom(parent.group, parent.artifact, parent.version));
        let mut effective = self.build_raw(raw, Vec::new()).await?;
        effective.parent = parent;
        Ok(effective)
    }

    /// Construct an effective model for a repository coordinate.
    ///
    /// # Errors
    ///
    /// Returns an error when repository access or effective-model construction fails.
    pub async fn build_coordinate(
        &self,
        coordinate: &Coordinate,
    ) -> Result<EffectivePom, ResolverError> {
        self.build_remote(coordinate.clone(), Vec::new()).await
    }

    pub(crate) fn remember(&self, model: &EffectivePom) {
        self.cache
            .lock()
            .expect("cache poisoned")
            .insert(model.coordinate.to_string(), model.clone());
    }

    fn build_remote(
        &self,
        coordinate: Coordinate,
        stack: Vec<String>,
    ) -> Pin<Box<dyn Future<Output = Result<EffectivePom, ResolverError>> + Send + '_>> {
        Box::pin(async move {
            let key = coordinate.to_string();
            if let Some(cached) = self.cache.lock().expect("cache poisoned").get(&key) {
                return Ok(cached.clone());
            }
            if stack.contains(&key) {
                return Err(ResolverError::ModelCycle(
                    stack
                        .iter()
                        .chain(std::iter::once(&key))
                        .cloned()
                        .collect::<Vec<_>>()
                        .join(" -> "),
                ));
            }
            let mut next_stack = stack;
            next_stack.push(key.clone());
            let xml = self.source.load_pom(&coordinate).await?;
            let effective =
                self.build_raw(parse_pom(&xml)?, next_stack)
                    .await
                    .map_err(|source| ResolverError::ModelContext {
                        coordinate: key.clone(),
                        source: Box::new(source),
                    })?;
            self.cache
                .lock()
                .expect("cache poisoned")
                .insert(key, effective.clone());
            Ok(effective)
        })
    }

    #[allow(clippy::too_many_lines)]
    fn build_raw(
        &self,
        raw: RawPom,
        stack: Vec<String>,
    ) -> Pin<Box<dyn Future<Output = Result<EffectivePom, ResolverError>> + Send + '_>> {
        Box::pin(async move {
            let parent_coordinate = raw.parent.as_ref().map(|parent| {
                Coordinate::pom(
                    interpolate_loose(&parent.group, &raw.properties),
                    interpolate_loose(&parent.artifact, &raw.properties),
                    interpolate_loose(&parent.version, &raw.properties),
                )
            });
            let parent = if let Some(coordinate) = &parent_coordinate {
                Some(self.build_remote(coordinate.clone(), stack.clone()).await?)
            } else {
                None
            };

            let group = raw
                .group
                .clone()
                .or_else(|| parent.as_ref().map(|model| model.coordinate.group.clone()))
                .ok_or_else(|| ResolverError::InvalidPom("missing project groupId".to_owned()))?;
            let version = raw
                .version
                .clone()
                .or_else(|| {
                    parent
                        .as_ref()
                        .map(|model| model.coordinate.version.clone())
                })
                .ok_or_else(|| ResolverError::InvalidPom("missing project version".to_owned()))?;

            let mut property_templates = parent
                .as_ref()
                .map_or_else(BTreeMap::new, |model| model.property_templates.clone());
            property_templates.extend(raw.properties.clone());
            let mut properties = property_templates.clone();
            insert_platform_properties(&mut properties);
            insert_project_properties(
                &mut properties,
                &group,
                &raw.artifact,
                &version,
                raw.parent.as_ref(),
            );
            resolve_property_map(&mut properties);

            let group = interpolate_required(&group, &properties)?;
            let artifact = interpolate_required(&raw.artifact, &properties)?;
            let version = interpolate_required(&version, &properties)?;
            insert_project_properties(
                &mut properties,
                &group,
                &artifact,
                &version,
                raw.parent.as_ref(),
            );
            resolve_property_map(&mut properties);

            let mut current_management = DependencyCollection::default();
            let mut direct_management = Vec::new();
            let mut imports = Vec::new();
            for (index, dependency) in raw.dependency_management.iter().enumerate() {
                let dependency = interpolate_managed_dependency(dependency, &properties);
                if dependency.scope == "import" && dependency.dependency_type == "pom" {
                    imports.push((index, dependency.coordinate()?));
                } else {
                    direct_management.push(dependency);
                }
            }
            let bom_imports = imports
                .iter()
                .map(|(_, coordinate)| coordinate.clone())
                .collect();
            let import_futures = imports.into_iter().map(|(index, coordinate)| {
                let import_stack = stack.clone();
                async move {
                    self.build_remote(coordinate, import_stack)
                        .await
                        .map(|model| (index, model))
                }
            });
            let mut imported_models = stream::iter(import_futures)
                .buffer_unordered(32)
                .try_collect::<Vec<_>>()
                .await?;
            imported_models.sort_unstable_by_key(|(index, _)| *index);
            for (_, imported) in imported_models {
                for managed in imported.dependency_management {
                    current_management.insert_if_absent(managed);
                }
            }
            for dependency in direct_management {
                current_management.replace(dependency);
            }
            let mut dependency_management = DependencyCollection::new(
                parent
                    .as_ref()
                    .map_or_else(Vec::new, |model| model.dependency_management.clone()),
            );
            for dependency in current_management.into_items() {
                dependency_management.replace(dependency);
            }

            let management_indices = dependency_management.indices.clone();
            let dependency_management = dependency_management.into_items();
            let mut dependencies = DependencyCollection::new(
                parent
                    .as_ref()
                    .map_or_else(Vec::new, |model| model.dependencies.clone()),
            );
            for dependency in raw.dependencies {
                let mut dependency = interpolate_dependency(&dependency, &properties)?;
                apply_management(&mut dependency, &dependency_management, &management_indices);
                dependencies.replace(dependency);
            }

            let mut repositories = parent
                .as_ref()
                .map_or_else(Vec::new, |model| model.repositories.clone());
            for repository in raw.repositories {
                replace_repository(
                    &mut repositories,
                    Repository {
                        id: interpolate_required(&repository.id, &properties)?,
                        url: interpolate_required(&repository.url, &properties)?,
                        ..repository
                    },
                );
            }

            let annotation_processors = raw
                .annotation_processors
                .into_iter()
                .map(|processor| {
                    let mut dependency =
                        interpolate_dependency(&processor.dependency, &properties)?;
                    apply_management(&mut dependency, &dependency_management, &management_indices);
                    Ok(AnnotationProcessor { dependency })
                })
                .collect::<Result<Vec<_>, ResolverError>>()?;
            let compiler_args = raw
                .compiler_args
                .iter()
                .map(|argument| interpolate_required(argument, &properties))
                .collect::<Result<Vec<_>, _>>()?;
            let modules = raw
                .modules
                .iter()
                .map(|module| interpolate_required(module, &properties))
                .collect::<Result<Vec<_>, _>>()?;
            let relocation = raw.relocation.map(|relocation| Coordinate {
                group: interpolate_loose(
                    relocation.group.as_deref().unwrap_or(&group),
                    &properties,
                ),
                artifact: interpolate_loose(
                    relocation.artifact.as_deref().unwrap_or(&artifact),
                    &properties,
                ),
                version: interpolate_loose(
                    relocation.version.as_deref().unwrap_or(&version),
                    &properties,
                ),
                extension: "pom".to_owned(),
                classifier: None,
            });

            Ok(EffectivePom {
                coordinate: Coordinate::pom(group, artifact, version),
                parent: parent_coordinate,
                packaging: interpolate_required(&raw.packaging, &properties)?,
                modules,
                property_templates,
                properties,
                dependencies: dependencies.into_items(),
                dependency_management,
                bom_imports,
                repositories,
                annotation_processors,
                compiler_args,
                relocation,
            })
        })
    }
}

fn insert_platform_properties(properties: &mut BTreeMap<String, String>) {
    let os = match std::env::consts::OS {
        "macos" => "osx",
        "windows" => "windows",
        "linux" => "linux",
        other => other,
    };
    let arch = match std::env::consts::ARCH {
        "x86_64" => "x86_64",
        "aarch64" => "aarch_64",
        "x86" => "x86_32",
        "arm" => "arm_32",
        other => other,
    };
    properties
        .entry("os.detected.name".to_owned())
        .or_insert_with(|| os.to_owned());
    properties
        .entry("os.detected.arch".to_owned())
        .or_insert_with(|| arch.to_owned());
    properties
        .entry("os.detected.classifier".to_owned())
        .or_insert_with(|| format!("{os}-{arch}"));
}

fn insert_project_properties(
    properties: &mut BTreeMap<String, String>,
    group: &str,
    artifact: &str,
    version: &str,
    parent: Option<&crate::Parent>,
) {
    for prefix in ["project", "pom"] {
        properties.insert(format!("{prefix}.groupId"), group.to_owned());
        properties.insert(format!("{prefix}.artifactId"), artifact.to_owned());
        properties.insert(format!("{prefix}.version"), version.to_owned());
    }
    properties.insert("groupId".to_owned(), group.to_owned());
    properties.insert("artifactId".to_owned(), artifact.to_owned());
    properties.insert("version".to_owned(), version.to_owned());
    if let Some(parent) = parent {
        properties.insert("project.parent.groupId".to_owned(), parent.group.clone());
        properties.insert(
            "project.parent.artifactId".to_owned(),
            parent.artifact.clone(),
        );
        properties.insert("project.parent.version".to_owned(), parent.version.clone());
    }
}

fn resolve_property_map(properties: &mut BTreeMap<String, String>) {
    for _ in 0..32 {
        let snapshot = properties.clone();
        let mut changed = false;
        for value in properties.values_mut() {
            let resolved = interpolate_loose(value, &snapshot);
            changed |= resolved != *value;
            *value = resolved;
        }
        if !changed {
            return;
        }
    }
}

fn interpolate_dependency(
    dependency: &Dependency,
    properties: &BTreeMap<String, String>,
) -> Result<Dependency, ResolverError> {
    Ok(Dependency {
        group: interpolate_required(&dependency.group, properties)?,
        artifact: interpolate_required(&dependency.artifact, properties)?,
        version: dependency
            .version
            .as_ref()
            .map(|value| interpolate_required(value, properties))
            .transpose()?,
        scope: interpolate_required(&dependency.scope, properties)?,
        dependency_type: interpolate_required(&dependency.dependency_type, properties)?,
        classifier: dependency
            .classifier
            .as_ref()
            .map(|value| interpolate_required(value, properties))
            .transpose()?,
        exclusions: dependency
            .exclusions
            .iter()
            .map(|exclusion| {
                Ok(crate::Exclusion {
                    group: interpolate_required(&exclusion.group, properties)?,
                    artifact: interpolate_required(&exclusion.artifact, properties)?,
                })
            })
            .collect::<Result<Vec<_>, ResolverError>>()?,
        ..dependency.clone()
    })
}

fn interpolate_managed_dependency(
    dependency: &Dependency,
    properties: &BTreeMap<String, String>,
) -> Dependency {
    Dependency {
        group: interpolate_loose(&dependency.group, properties),
        artifact: interpolate_loose(&dependency.artifact, properties),
        version: dependency
            .version
            .as_ref()
            .map(|value| interpolate_loose(value, properties)),
        scope: interpolate_loose(&dependency.scope, properties),
        dependency_type: interpolate_loose(&dependency.dependency_type, properties),
        classifier: dependency
            .classifier
            .as_ref()
            .map(|value| interpolate_loose(value, properties)),
        exclusions: dependency
            .exclusions
            .iter()
            .map(|exclusion| crate::Exclusion {
                group: interpolate_loose(&exclusion.group, properties),
                artifact: interpolate_loose(&exclusion.artifact, properties),
            })
            .collect(),
        ..dependency.clone()
    }
}

fn apply_management(
    dependency: &mut Dependency,
    management: &[Dependency],
    indices: &HashMap<String, usize>,
) {
    let key = dependency.management_key();
    let Some(managed) = indices.get(&key).map(|index| &management[*index]) else {
        return;
    };
    if dependency.version.is_none() {
        dependency.version.clone_from(&managed.version);
    }
    if !dependency.scope_explicit {
        dependency.scope.clone_from(&managed.scope);
    }
    if !dependency.type_explicit {
        dependency
            .dependency_type
            .clone_from(&managed.dependency_type);
    }
    if dependency.optional.is_none() {
        dependency.optional.clone_from(&managed.optional);
    }
    for exclusion in &managed.exclusions {
        if !dependency.exclusions.contains(exclusion) {
            dependency.exclusions.push(exclusion.clone());
        }
    }
}

#[derive(Default)]
struct DependencyCollection {
    items: Vec<Dependency>,
    indices: HashMap<String, usize>,
}

impl DependencyCollection {
    fn new(items: Vec<Dependency>) -> Self {
        let mut indices = HashMap::with_capacity(items.len());
        for (index, dependency) in items.iter().enumerate() {
            indices.entry(dependency.management_key()).or_insert(index);
        }
        Self { items, indices }
    }

    fn insert_if_absent(&mut self, dependency: Dependency) {
        let key = dependency.management_key();
        if let std::collections::hash_map::Entry::Vacant(entry) = self.indices.entry(key) {
            entry.insert(self.items.len());
            self.items.push(dependency);
        }
    }

    fn replace(&mut self, dependency: Dependency) {
        let key = dependency.management_key();
        if let Some(index) = self.indices.get(&key) {
            self.items[*index] = dependency;
        } else {
            self.indices.insert(key, self.items.len());
            self.items.push(dependency);
        }
    }

    fn into_items(self) -> Vec<Dependency> {
        self.items
    }
}

fn replace_repository(items: &mut Vec<Repository>, repository: Repository) {
    if let Some(existing) = items.iter_mut().find(|item| item.id == repository.id) {
        *existing = repository;
    } else {
        items.push(repository);
    }
}

fn interpolate_required(
    value: &str,
    properties: &BTreeMap<String, String>,
) -> Result<String, ResolverError> {
    let result = interpolate_loose(value, properties);
    if let Some(property) = first_property(&result) {
        Err(ResolverError::UnresolvedProperty {
            property,
            value: value.to_owned(),
        })
    } else {
        Ok(result)
    }
}

fn interpolate_loose(value: &str, properties: &BTreeMap<String, String>) -> String {
    let mut result = value.to_owned();
    for _ in 0..32 {
        let Some(property) = first_property(&result) else {
            break;
        };
        let Some(replacement) = properties.get(&property) else {
            break;
        };
        let marker = format!("${{{property}}}");
        result = result.replace(&marker, replacement);
    }
    result
}

fn first_property(value: &str) -> Option<String> {
    let start = value.find("${")?;
    let remainder = &value[start + 2..];
    let end = remainder.find('}')?;
    Some(remainder[..end].to_owned())
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    struct MemorySource(HashMap<String, String>);

    impl PomSource for MemorySource {
        fn load_pom(
            &self,
            coordinate: &Coordinate,
        ) -> Pin<Box<dyn Future<Output = Result<String, ResolverError>> + Send + '_>> {
            let key = coordinate.to_string();
            Box::pin(async move {
                self.0.get(&key).cloned().ok_or(ResolverError::HttpStatus {
                    url: key,
                    status: reqwest::StatusCode::NOT_FOUND,
                })
            })
        }
    }

    #[tokio::test]
    async fn inherits_properties_and_imports_bom_management() {
        let source = MemorySource(HashMap::from([
            (
                "com.example:parent:pom:1".to_owned(),
                pom(
                    "<groupId>com.example</groupId><artifactId>parent</artifactId><version>1</version>
                     <properties><java.version>17</java.version>
                     <maven.compiler.release>${java.version}</maven.compiler.release></properties>
                     <dependencyManagement><dependencies><dependency>
                     <groupId>com.example</groupId><artifactId>bom</artifactId><version>1</version>
                     <type>pom</type><scope>import</scope></dependency></dependencies></dependencyManagement>",
                ),
            ),
            (
                "com.example:bom:pom:1".to_owned(),
                pom(
                    "<groupId>com.example</groupId><artifactId>bom</artifactId><version>1</version>
                     <properties><lib.version>2</lib.version></properties>
                     <dependencyManagement><dependencies><dependency>
                     <groupId>org.example</groupId><artifactId>library</artifactId>
                     <version>${lib.version}</version></dependency></dependencies></dependencyManagement>",
                ),
            ),
        ]));
        let builder = EffectiveModelBuilder::new(source);
        let effective = builder
            .build_local(&pom(
                "<parent><groupId>com.example</groupId><artifactId>parent</artifactId><version>1</version></parent>
                 <artifactId>app</artifactId><properties><java.version>25</java.version></properties>
                 <dependencies><dependency>
                 <groupId>org.example</groupId><artifactId>library</artifactId>
                 </dependency></dependencies>",
            ))
            .await
            .expect("effective model");

        assert_eq!(effective.coordinate.to_string(), "com.example:app:pom:1");
        assert_eq!(effective.dependencies[0].version.as_deref(), Some("2"));
        assert_eq!(effective.properties["maven.compiler.release"], "25");
    }

    #[tokio::test]
    async fn concurrent_bom_imports_preserve_declaration_order() {
        let source = MemorySource(HashMap::from([
            (
                "com.example:first:pom:1".to_owned(),
                pom(
                    "<groupId>com.example</groupId><artifactId>first</artifactId><version>1</version>
                     <dependencyManagement><dependencies><dependency>
                     <groupId>org.example</groupId><artifactId>library</artifactId>
                     <version>1</version></dependency></dependencies></dependencyManagement>",
                ),
            ),
            (
                "com.example:second:pom:1".to_owned(),
                pom(
                    "<groupId>com.example</groupId><artifactId>second</artifactId><version>1</version>
                     <dependencyManagement><dependencies><dependency>
                     <groupId>org.example</groupId><artifactId>library</artifactId>
                     <version>2</version></dependency></dependencies></dependencyManagement>",
                ),
            ),
        ]));
        let builder = EffectiveModelBuilder::new(source);
        let effective = builder
            .build_local(&pom(
                "<groupId>com.example</groupId><artifactId>app</artifactId><version>1</version>
                 <dependencyManagement><dependencies>
                 <dependency><groupId>com.example</groupId><artifactId>first</artifactId>
                 <version>1</version><type>pom</type><scope>import</scope></dependency>
                 <dependency><groupId>com.example</groupId><artifactId>second</artifactId>
                 <version>1</version><type>pom</type><scope>import</scope></dependency>
                 </dependencies></dependencyManagement>",
            ))
            .await
            .expect("effective model");

        assert_eq!(
            effective.dependency_management[0].version.as_deref(),
            Some("1")
        );
    }

    #[tokio::test]
    async fn unused_unresolved_dependency_management_property_is_tolerated() {
        let builder = EffectiveModelBuilder::new(MemorySource(HashMap::new()));
        let effective = builder
            .build_local(&pom(
                "<groupId>com.example</groupId><artifactId>parent</artifactId><version>1</version>
                 <dependencyManagement><dependencies><dependency>
                 <groupId>org.codehaus.groovy</groupId><artifactId>groovy</artifactId>
                 <version>${groovy.version}</version></dependency>
                 </dependencies></dependencyManagement>",
            ))
            .await
            .expect("unused unresolved management entry");

        assert_eq!(
            effective.dependency_management[0].version.as_deref(),
            Some("${groovy.version}")
        );
    }

    #[tokio::test]
    async fn dependency_management_applies_optional_and_exclusions() {
        let builder = EffectiveModelBuilder::new(MemorySource(HashMap::new()));
        let effective = builder
            .build_local(&pom(
                "<groupId>com.example</groupId><artifactId>app</artifactId><version>1</version>
                 <dependencyManagement><dependencies><dependency>
                 <groupId>org.example</groupId><artifactId>library</artifactId><version>2</version>
                 <optional>true</optional><exclusions><exclusion><groupId>org.unwanted</groupId>
                 <artifactId>support</artifactId></exclusion></exclusions>
                 </dependency></dependencies></dependencyManagement>
                 <dependencies><dependency><groupId>org.example</groupId>
                 <artifactId>library</artifactId></dependency></dependencies>",
            ))
            .await
            .expect("effective model");

        assert_eq!(effective.dependencies[0].optional, Some(true));
        assert_eq!(effective.dependencies[0].exclusions[0].artifact, "support");
    }

    fn pom(body: &str) -> String {
        format!("<project><modelVersion>4.0.0</modelVersion>{body}</project>")
    }
}
