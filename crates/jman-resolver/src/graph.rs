use std::{
    collections::{BTreeSet, HashMap, HashSet, VecDeque},
    path::Path,
    sync::Arc,
};

use futures::{stream, StreamExt};
use sha2::{Digest, Sha256};

use crate::{
    CachedFile, Coordinate, Dependency, EffectiveModelBuilder, EffectivePom, Exclusion,
    RepositoryClient, ResolverError,
};

type ArtifactTask = Result<(usize, CachedFile), ResolverError>;

#[derive(Clone, Debug)]
pub struct ResolvedPackage {
    pub coordinate: Coordinate,
    pub source: String,
    pub pom_checksum: String,
    pub artifact_checksum: Option<String>,
    pub artifact_size: Option<u64>,
    pub artifact_path: Option<std::path::PathBuf>,
    pub dependencies: Vec<String>,
    pub effective_scopes: BTreeSet<String>,
    pub selected_parent: Option<usize>,
}

#[derive(Clone, Debug, Default)]
pub struct ResolvedGraph {
    pub packages: Vec<ResolvedPackage>,
    pub compile_classpath: Vec<String>,
    pub runtime_classpath: Vec<String>,
    pub test_classpath: Vec<String>,
    pub processor_classpath: Vec<String>,
}

pub struct DependencyResolver {
    repository: RepositoryClient,
}

impl DependencyResolver {
    #[must_use]
    pub fn new(repository: RepositoryClient) -> Self {
        Self { repository }
    }

    /// Load a local POM and construct its effective dependency model.
    ///
    /// # Errors
    ///
    /// Returns an error when the POM cannot be read or modeled.
    pub async fn import_project(&self, pom_path: &Path) -> Result<EffectivePom, ResolverError> {
        self.import_workspace(pom_path)
            .await?
            .into_iter()
            .next()
            .map(|project| project.effective)
            .ok_or_else(|| ResolverError::InvalidPom("empty Maven reactor".to_owned()))
    }

    /// Discover a Maven reactor and construct local-parent-aware effective models.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid module paths, duplicate modules, missing
    /// POMs, or effective-model failures.
    pub async fn import_workspace(
        &self,
        pom_path: &Path,
    ) -> Result<Vec<crate::MavenProject>, ResolverError> {
        let root_pom =
            tokio::fs::canonicalize(pom_path)
                .await
                .map_err(|source| ResolverError::Cache {
                    path: pom_path.to_owned(),
                    source,
                })?;
        let root_directory = root_pom
            .parent()
            .ok_or_else(|| {
                ResolverError::InvalidPom("reactor root POM has no parent directory".to_owned())
            })?
            .to_owned();
        let builder = EffectiveModelBuilder::new(self.repository.clone());
        let mut pending = VecDeque::from([root_pom]);
        let mut discovered = HashSet::new();
        let mut projects = Vec::new();
        while let Some(module_pom) = pending.pop_front() {
            if !discovered.insert(module_pom.clone()) {
                return Err(ResolverError::InvalidPom(format!(
                    "duplicate or cyclic reactor module {}",
                    module_pom.display()
                )));
            }
            let xml = tokio::fs::read_to_string(&module_pom)
                .await
                .map_err(|source| ResolverError::Cache {
                    path: module_pom.clone(),
                    source,
                })?;
            let raw = crate::parse_pom(&xml)?;
            let repository_only_parent = raw
                .parent
                .as_ref()
                .and_then(|parent| parent.relative_path.as_deref())
                == Some("");
            let effective = if repository_only_parent {
                EffectiveModelBuilder::new(self.repository.clone())
                    .build_local(&xml)
                    .await?
            } else {
                builder.build_local(&xml).await?
            };
            builder.remember(&effective);
            let module_directory = module_pom.parent().ok_or_else(|| {
                ResolverError::InvalidPom(format!(
                    "module POM {} has no parent directory",
                    module_pom.display()
                ))
            })?;
            for module in &effective.modules {
                let candidate = module_directory.join(module);
                let candidate_pom = if candidate.file_name().is_some_and(|name| name == "pom.xml") {
                    candidate
                } else {
                    candidate.join("pom.xml")
                };
                let canonical =
                    tokio::fs::canonicalize(&candidate_pom)
                        .await
                        .map_err(|source| ResolverError::Cache {
                            path: candidate_pom,
                            source,
                        })?;
                if !canonical.starts_with(&root_directory) {
                    return Err(ResolverError::InvalidPom(format!(
                        "reactor module {} escapes workspace {}",
                        canonical.display(),
                        root_directory.display()
                    )));
                }
                pending.push_back(canonical);
            }
            projects.push(crate::MavenProject {
                source: module_pom,
                effective,
            });
        }
        Ok(projects)
    }

    /// Import the effective reactor XML produced by Maven's help plugin.
    ///
    /// Maven has already applied project inheritance, profiles, settings, and
    /// plugin management. JMAN therefore treats each exported project as a
    /// precomputed model while retaining its parent coordinate as provenance.
    ///
    /// # Errors
    ///
    /// Returns an error for malformed exports, projects outside the requested
    /// workspace, missing source POMs, or invalid effective metadata.
    pub async fn import_effective_workspace(
        &self,
        xml: &str,
        workspace_root: &Path,
    ) -> Result<Vec<crate::MavenProject>, ResolverError> {
        let workspace_root = tokio::fs::canonicalize(workspace_root)
            .await
            .map_err(|source| ResolverError::Cache {
                path: workspace_root.to_owned(),
                source,
            })?;
        let exported = crate::pom::parse_effective_workspace(xml)?;
        let builder = EffectiveModelBuilder::new(self.repository.clone());
        let mut projects = Vec::with_capacity(exported.len());
        let mut sources = HashSet::new();
        for project in exported {
            let directory =
                tokio::fs::canonicalize(&project.directory)
                    .await
                    .map_err(|source| ResolverError::Cache {
                        path: project.directory.clone(),
                        source,
                    })?;
            if !directory.starts_with(&workspace_root) {
                return Err(ResolverError::InvalidPom(format!(
                    "effective Maven project {} escapes workspace {}",
                    directory.display(),
                    workspace_root.display()
                )));
            }
            let source = tokio::fs::canonicalize(directory.join("pom.xml"))
                .await
                .map_err(|source| ResolverError::Cache {
                    path: directory.join("pom.xml"),
                    source,
                })?;
            if !sources.insert(source.clone()) {
                return Err(ResolverError::InvalidPom(format!(
                    "effective Maven reactor contains duplicate project {}",
                    source.display()
                )));
            }
            let effective = builder.build_precomputed(project.raw).await?;
            builder.remember(&effective);
            projects.push(crate::MavenProject { source, effective });
        }
        Ok(projects)
    }

    /// Resolve, mediate, and download all project dependency roles.
    ///
    /// # Errors
    ///
    /// Returns an error for incompatible metadata, missing artifacts, or cache failures.
    pub async fn resolve(&self, project: &EffectivePom) -> Result<ResolvedGraph, ResolverError> {
        self.resolve_in_workspace(project, &[]).await
    }

    /// Resolve one module with unpublished reactor modules available locally.
    ///
    /// # Errors
    ///
    /// Returns an error for incompatible metadata, missing artifacts, or cache failures.
    #[allow(clippy::too_many_lines)]
    pub async fn resolve_in_workspace(
        &self,
        project: &EffectivePom,
        workspace: &[crate::MavenProject],
    ) -> Result<ResolvedGraph, ResolverError> {
        let builder = EffectiveModelBuilder::new(self.repository.clone());
        for module in workspace {
            builder.remember(&module.effective);
        }
        let mut queue = root_candidates(project);

        let mut graph = ResolvedGraph::default();
        let mut selected: HashMap<String, usize> = HashMap::new();
        let mut prefetched_poms = HashSet::new();
        let artifact_limit = Arc::new(tokio::sync::Semaphore::new(32));
        let mut artifact_downloads = tokio::task::JoinSet::new();
        self.prefetch_candidate_poms(&queue, &mut prefetched_poms)
            .await;
        while let Some(candidate) = queue.pop_front() {
            if candidate
                .active_exclusions
                .iter()
                .any(|exclusion| exclusion.matches(&candidate.dependency))
            {
                continue;
            }

            let coordinate = candidate.dependency.coordinate()?;
            let pom_coordinate = Coordinate::pom(
                coordinate.group.clone(),
                coordinate.artifact.clone(),
                coordinate.version.clone(),
            );
            let effective = builder.build_coordinate(&pom_coordinate).await?;
            if let Some(relocation) = &effective.relocation {
                if relocation.group != coordinate.group
                    || relocation.artifact != coordinate.artifact
                    || relocation.version != coordinate.version
                {
                    let mut relocated = candidate;
                    relocated.dependency.group.clone_from(&relocation.group);
                    relocated
                        .dependency
                        .artifact
                        .clone_from(&relocation.artifact);
                    relocated.dependency.version = Some(relocation.version.clone());
                    queue.push_front(relocated);
                    continue;
                }
            }
            let key = candidate.dependency.management_key();
            if let Some(&index) = selected.get(&key) {
                add_edge(&mut graph.packages, candidate.parent, index);
                if !graph.packages[index]
                    .effective_scopes
                    .insert(candidate.scope.clone())
                {
                    continue;
                }
                let selected_coordinate = &graph.packages[index].coordinate;
                let effective = if selected_coordinate.version == coordinate.version {
                    effective
                } else {
                    builder
                        .build_coordinate(&Coordinate::pom(
                            selected_coordinate.group.clone(),
                            selected_coordinate.artifact.clone(),
                            selected_coordinate.version.clone(),
                        ))
                        .await?
                };
                enqueue_children(
                    &mut queue,
                    effective,
                    &candidate,
                    index,
                    &project.dependency_management,
                );
            } else {
                let local_module = workspace.iter().find(|module| {
                    module.effective.coordinate.group == coordinate.group
                        && module.effective.coordinate.artifact == coordinate.artifact
                        && module.effective.coordinate.version == coordinate.version
                });
                let pom_file = if let Some(module) = local_module {
                    local_pom_file(module).await?
                } else {
                    self.repository.fetch(&pom_coordinate).await?
                };
                let index = graph.packages.len();
                selected.insert(key, index);
                if local_module.is_none() {
                    self.schedule_artifact(
                        &coordinate,
                        index,
                        &artifact_limit,
                        &mut artifact_downloads,
                    );
                }
                graph.packages.push(ResolvedPackage {
                    coordinate,
                    source: pom_file.source,
                    pom_checksum: pom_file.checksum,
                    artifact_checksum: None,
                    artifact_size: None,
                    artifact_path: None,
                    dependencies: Vec::new(),
                    effective_scopes: BTreeSet::from([candidate.scope.clone()]),
                    selected_parent: candidate.parent,
                });
                add_edge(&mut graph.packages, candidate.parent, index);
                enqueue_children(
                    &mut queue,
                    effective,
                    &candidate,
                    index,
                    &project.dependency_management,
                );
            }
            self.prefetch_candidate_poms(&queue, &mut prefetched_poms)
                .await;
        }

        finish_artifacts(&mut artifact_downloads, &mut graph).await?;
        build_classpaths(&mut graph);
        Ok(graph)
    }

    async fn prefetch_candidate_poms(
        &self,
        queue: &VecDeque<Candidate>,
        prefetched: &mut HashSet<String>,
    ) {
        const MAX_CONCURRENT_POM_FETCHES: usize = 32;

        let coordinates = queue
            .iter()
            .filter(|candidate| {
                !candidate
                    .active_exclusions
                    .iter()
                    .any(|exclusion| exclusion.matches(&candidate.dependency))
            })
            .filter_map(|candidate| candidate.dependency.coordinate().ok())
            .map(|coordinate| {
                Coordinate::pom(coordinate.group, coordinate.artifact, coordinate.version)
            })
            .filter(|coordinate| prefetched.insert(coordinate.to_string()))
            .collect::<Vec<_>>();
        let fetches = coordinates.into_iter().map(|coordinate| {
            let repository = self.repository.clone();
            async move {
                // Prefetch is speculative: resolution must only surface an
                // error if Maven mediation actually selects this candidate.
                let _ = repository.fetch(&coordinate).await;
            }
        });
        stream::iter(fetches)
            .buffer_unordered(MAX_CONCURRENT_POM_FETCHES)
            .collect::<Vec<_>>()
            .await;
    }

    fn schedule_artifact(
        &self,
        coordinate: &Coordinate,
        index: usize,
        limit: &Arc<tokio::sync::Semaphore>,
        downloads: &mut tokio::task::JoinSet<ArtifactTask>,
    ) {
        if coordinate.extension == "pom" {
            return;
        }
        let repository = self.repository.clone();
        let coordinate = coordinate.clone();
        let limit = Arc::clone(limit);
        downloads.spawn(async move {
            let _permit = limit
                .acquire_owned()
                .await
                .map_err(|error| ResolverError::BackgroundTask(error.to_string()))?;
            repository
                .fetch(&coordinate)
                .await
                .map(|artifact| (index, artifact))
        });
    }
}

async fn local_pom_file(module: &crate::MavenProject) -> Result<CachedFile, ResolverError> {
    let bytes = tokio::fs::read(&module.source)
        .await
        .map_err(|source| ResolverError::Cache {
            path: module.source.clone(),
            source,
        })?;
    Ok(CachedFile {
        checksum: format!("sha256:{}", hex::encode(Sha256::digest(&bytes))),
        bytes,
        path: module.source.clone(),
        source: format!("workspace:{}", module.source.display()),
    })
}

fn root_candidates(project: &EffectivePom) -> VecDeque<Candidate> {
    let dependencies = project.dependencies.iter().map(|dependency| Candidate {
        dependency: dependency.clone(),
        scope: dependency.scope.clone(),
        active_exclusions: Vec::new(),
        descendant_exclusions: dependency.exclusions.clone(),
        parent: None,
    });
    let processors = project
        .annotation_processors
        .iter()
        .map(|processor| Candidate {
            dependency: processor.dependency.clone(),
            scope: "processor".to_owned(),
            active_exclusions: Vec::new(),
            descendant_exclusions: processor.dependency.exclusions.clone(),
            parent: None,
        });
    dependencies.chain(processors).collect()
}

async fn finish_artifacts(
    downloads: &mut tokio::task::JoinSet<ArtifactTask>,
    graph: &mut ResolvedGraph,
) -> Result<(), ResolverError> {
    let mut artifacts = Vec::new();
    while let Some(result) = downloads.join_next().await {
        artifacts.push(result.map_err(|error| ResolverError::BackgroundTask(error.to_string()))??);
    }
    artifacts.sort_unstable_by_key(|(index, _)| *index);
    for (index, artifact) in artifacts {
        let package = &mut graph.packages[index];
        package.artifact_checksum = Some(artifact.checksum);
        package.artifact_size = Some(artifact.bytes.len() as u64);
        package.artifact_path = Some(artifact.path);
    }
    Ok(())
}

fn enqueue_children(
    queue: &mut VecDeque<Candidate>,
    effective: EffectivePom,
    candidate: &Candidate,
    parent: usize,
    root_management: &[Dependency],
) {
    for mut dependency in effective.dependencies {
        if dependency.optional == Some(true) {
            continue;
        }
        apply_root_management(&mut dependency, root_management);
        let Some(scope) = derive_scope(&candidate.scope, &dependency.scope) else {
            continue;
        };
        let active_exclusions = candidate.descendant_exclusions.clone();
        let mut descendant_exclusions = active_exclusions.clone();
        descendant_exclusions.extend(dependency.exclusions.clone());
        queue.push_back(Candidate {
            dependency,
            scope,
            active_exclusions,
            descendant_exclusions,
            parent: Some(parent),
        });
    }
}

struct Candidate {
    dependency: Dependency,
    scope: String,
    active_exclusions: Vec<Exclusion>,
    descendant_exclusions: Vec<Exclusion>,
    parent: Option<usize>,
}

fn apply_root_management(dependency: &mut Dependency, management: &[Dependency]) {
    let Some(managed) = management
        .iter()
        .find(|managed| managed.management_key() == dependency.management_key())
    else {
        return;
    };
    if managed.version.is_some() {
        dependency.version.clone_from(&managed.version);
    }
    if managed.scope_explicit {
        dependency.scope.clone_from(&managed.scope);
    }
    if managed.type_explicit {
        dependency
            .dependency_type
            .clone_from(&managed.dependency_type);
    }
    for exclusion in &managed.exclusions {
        if !dependency.exclusions.contains(exclusion) {
            dependency.exclusions.push(exclusion.clone());
        }
    }
}

fn derive_scope(parent: &str, child: &str) -> Option<String> {
    match (parent, child) {
        ("compile", "compile") => Some("compile"),
        ("compile", "runtime") | ("runtime", "compile" | "runtime") => Some("runtime"),
        ("provided", "compile" | "runtime") => Some("provided"),
        ("test", "compile" | "runtime") => Some("test"),
        ("processor", "compile" | "runtime") => Some("processor"),
        _ => None,
    }
    .map(ToOwned::to_owned)
}

fn add_edge(packages: &mut [ResolvedPackage], parent: Option<usize>, child: usize) {
    let Some(parent) = parent else {
        return;
    };
    let coordinate = packages[child].coordinate.to_string();
    if !packages[parent].dependencies.contains(&coordinate) {
        packages[parent].dependencies.push(coordinate);
    }
}

fn build_classpaths(graph: &mut ResolvedGraph) {
    let mut compile = HashSet::new();
    let mut runtime = HashSet::new();
    let mut test = HashSet::new();
    let mut processors = HashSet::new();

    for package in &graph.packages {
        let Some(checksum) = &package.artifact_checksum else {
            continue;
        };
        for scope in &package.effective_scopes {
            match scope.as_str() {
                "compile" => {
                    push_unique(&mut graph.compile_classpath, &mut compile, checksum);
                    push_unique(&mut graph.runtime_classpath, &mut runtime, checksum);
                    push_unique(&mut graph.test_classpath, &mut test, checksum);
                }
                "runtime" => {
                    push_unique(&mut graph.runtime_classpath, &mut runtime, checksum);
                    push_unique(&mut graph.test_classpath, &mut test, checksum);
                }
                "provided" => {
                    push_unique(&mut graph.compile_classpath, &mut compile, checksum);
                    push_unique(&mut graph.test_classpath, &mut test, checksum);
                }
                "test" => push_unique(&mut graph.test_classpath, &mut test, checksum),
                "processor" => {
                    push_unique(&mut graph.processor_classpath, &mut processors, checksum);
                }
                _ => {}
            }
        }
    }
}

fn push_unique(target: &mut Vec<String>, seen: &mut HashSet<String>, checksum: &str) {
    if seen.insert(checksum.to_owned()) {
        target.push(checksum.to_owned());
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, path::Path};

    use super::*;

    #[test]
    fn applies_maven_scope_propagation() {
        assert_eq!(
            derive_scope("compile", "compile").as_deref(),
            Some("compile")
        );
        assert_eq!(
            derive_scope("compile", "runtime").as_deref(),
            Some("runtime")
        );
        assert_eq!(
            derive_scope("runtime", "compile").as_deref(),
            Some("runtime")
        );
        assert_eq!(
            derive_scope("provided", "runtime").as_deref(),
            Some("provided")
        );
        assert_eq!(derive_scope("compile", "provided"), None);
        assert_eq!(derive_scope("test", "test"), None);
    }

    #[test]
    fn root_management_overrides_a_transitive_version() {
        let mut dependency = make_dependency("org.example", "library", "1");
        let managed = make_dependency("org.example", "library", "2");

        apply_root_management(&mut dependency, &[managed]);

        assert_eq!(dependency.version.as_deref(), Some("2"));
    }

    #[test]
    fn root_management_preserves_dependency_specific_exclusions() {
        let mut dependency = make_dependency("org.example", "library", "1");
        dependency.exclusions.push(Exclusion {
            group: "org.unwanted".to_owned(),
            artifact: "specific".to_owned(),
        });
        let mut managed = make_dependency("org.example", "library", "2");
        managed.exclusions.push(Exclusion {
            group: "org.unwanted".to_owned(),
            artifact: "managed".to_owned(),
        });

        apply_root_management(&mut dependency, &[managed]);

        assert_eq!(dependency.exclusions[0].artifact, "specific");
        assert_eq!(dependency.exclusions[1].artifact, "managed");
    }

    fn make_dependency(group: &str, artifact: &str, version: &str) -> Dependency {
        Dependency {
            group: group.to_owned(),
            artifact: artifact.to_owned(),
            version: Some(version.to_owned()),
            scope: "compile".to_owned(),
            scope_explicit: false,
            dependency_type: "jar".to_owned(),
            type_explicit: false,
            classifier: None,
            optional: None,
            exclusions: Vec::new(),
        }
    }

    #[tokio::test]
    async fn ignores_compiler_plugin_processors_during_resolution() {
        let temporary = tempfile::tempdir().expect("temporary directory");
        let repository_root = temporary.path().join("repository");
        let cache = temporary.path().join("cache");
        write_artifact(
            &repository_root,
            "org.example",
            "processor",
            "1",
            "pom",
            &pom(
                "<groupId>org.example</groupId><artifactId>processor</artifactId><version>1</version>
                 <dependencies><dependency><groupId>org.example</groupId>
                 <artifactId>support</artifactId><version>1</version></dependency></dependencies>",
            ),
        );
        write_artifact(
            &repository_root,
            "org.example",
            "processor",
            "1",
            "jar",
            "processor",
        );
        write_artifact(
            &repository_root,
            "org.example",
            "support",
            "1",
            "pom",
            &pom("<groupId>org.example</groupId><artifactId>support</artifactId><version>1</version>"),
        );
        write_artifact(
            &repository_root,
            "org.example",
            "support",
            "1",
            "jar",
            "support",
        );
        let root_pom = temporary.path().join("pom.xml");
        fs::write(
            &root_pom,
            pom(
                "<groupId>com.example</groupId><artifactId>app</artifactId><version>1</version>
                 <dependencies><dependency><groupId>org.example</groupId>
                 <artifactId>processor</artifactId><version>1</version></dependency></dependencies>
                 <build><plugins><plugin><artifactId>maven-compiler-plugin</artifactId>
                 <configuration><annotationProcessorPaths><path><groupId>org.example</groupId>
                 <artifactId>processor</artifactId><version>1</version></path>
                 </annotationProcessorPaths></configuration></plugin></plugins></build>",
            ),
        )
        .expect("root POM");
        let repository =
            RepositoryClient::new(cache, vec![format!("file://{}", repository_root.display())])
                .expect("repository");
        let resolver = DependencyResolver::new(repository);
        let project = resolver
            .import_project(&root_pom)
            .await
            .expect("effective model");

        let graph = resolver.resolve(&project).await.expect("resolved graph");

        assert_eq!(graph.packages.len(), 2);
        assert_eq!(graph.compile_classpath.len(), 2);
        assert!(graph.processor_classpath.is_empty());
        assert!(temporary.path().join("cache/artifacts").is_dir());
    }

    #[tokio::test]
    async fn imports_reactor_children_with_an_unpublished_local_parent() {
        let temporary = tempfile::tempdir().expect("temporary directory");
        let child = temporary.path().join("child");
        fs::create_dir(&child).expect("child directory");
        fs::write(
            temporary.path().join("pom.xml"),
            pom(
                "<groupId>com.example</groupId><artifactId>parent</artifactId><version>1</version>
                 <packaging>pom</packaging><properties><library.version>2</library.version></properties>
                 <modules><module>child</module></modules>
                 <dependencyManagement><dependencies><dependency><groupId>org.example</groupId>
                 <artifactId>library</artifactId><version>${library.version}</version>
                 </dependency></dependencies></dependencyManagement>",
            ),
        )
        .expect("root POM");
        fs::write(
            child.join("pom.xml"),
            pom(
                "<parent><groupId>com.example</groupId><artifactId>parent</artifactId>
                 <version>1</version></parent><artifactId>child</artifactId>
                 <dependencies><dependency><groupId>org.example</groupId>
                 <artifactId>library</artifactId></dependency></dependencies>",
            ),
        )
        .expect("child POM");
        let repository = RepositoryClient::new(
            temporary.path().join("cache"),
            vec![format!(
                "file://{}",
                temporary.path().join("empty").display()
            )],
        )
        .expect("repository");

        let projects = DependencyResolver::new(repository)
            .import_workspace(&temporary.path().join("pom.xml"))
            .await
            .expect("reactor");

        assert_eq!(projects.len(), 2);
        assert_eq!(projects[1].effective.coordinate.artifact, "child");
        assert_eq!(
            projects[1].effective.dependencies[0].version.as_deref(),
            Some("2")
        );
    }

    #[tokio::test]
    async fn imports_a_precomputed_maven_reactor_without_reapplying_profiles() {
        let temporary = tempfile::tempdir().expect("temporary directory");
        let child = temporary.path().join("child");
        fs::create_dir(&child).expect("child directory");
        fs::write(
            temporary.path().join("pom.xml"),
            pom(
                "<groupId>com.example</groupId><artifactId>parent</artifactId><version>1</version>",
            ),
        )
        .expect("root POM");
        fs::write(
            child.join("pom.xml"),
            pom(
                "<parent><groupId>com.example</groupId><artifactId>parent</artifactId>
                 <version>1</version></parent><artifactId>child</artifactId>",
            ),
        )
        .expect("child POM");
        let exported = format!(
            r"<projects>
              <project><modelVersion>4.0.0</modelVersion>
                <groupId>com.example</groupId><artifactId>parent</artifactId><version>1</version>
                <packaging>pom</packaging><modules><module>child</module></modules>
                <build><directory>{}/target</directory></build>
              </project>
              <project><modelVersion>4.0.0</modelVersion>
                <parent><groupId>com.example</groupId><artifactId>parent</artifactId>
                  <version>1</version></parent>
                <groupId>com.example</groupId><artifactId>child</artifactId><version>1</version>
                <dependencies><dependency><groupId>org.example</groupId>
                  <artifactId>library</artifactId><version>2</version></dependency></dependencies>
                <profiles><profile><activation><activeByDefault>true</activeByDefault></activation>
                  <dependencies><dependency><groupId>org.example</groupId>
                    <artifactId>must-not-be-reapplied</artifactId><version>1</version>
                  </dependency></dependencies></profile></profiles>
                <build><directory>{}/target</directory><plugins><plugin>
                  <artifactId>maven-compiler-plugin</artifactId><configuration>
                    <release>21</release><parameters>true</parameters>
                    <compilerArgs><arg>-Afixture=true</arg></compilerArgs>
                  </configuration></plugin></plugins></build>
              </project>
            </projects>",
            temporary.path().display(),
            child.display()
        );
        let repository = RepositoryClient::new(
            temporary.path().join("cache"),
            vec![format!(
                "file://{}",
                temporary.path().join("empty").display()
            )],
        )
        .expect("repository");

        let projects = DependencyResolver::new(repository)
            .import_effective_workspace(&exported, temporary.path())
            .await
            .expect("effective reactor");

        assert_eq!(projects.len(), 2);
        assert_eq!(projects[1].source, child.join("pom.xml"));
        assert_eq!(projects[1].effective.coordinate.artifact, "child");
        assert_eq!(projects[1].effective.dependencies.len(), 1);
        assert_eq!(projects[1].effective.dependencies[0].artifact, "library");
        assert_eq!(
            projects[1]
                .effective
                .properties
                .get("maven.compiler.release")
                .map(String::as_str),
            Some("21")
        );
        assert_eq!(
            projects[1].effective.compiler_args,
            ["-parameters", "-Afixture=true"]
        );
    }

    #[tokio::test]
    async fn rejects_reactor_modules_outside_the_workspace() {
        let temporary = tempfile::tempdir().expect("temporary directory");
        let workspace = temporary.path().join("workspace");
        let outside = temporary.path().join("outside");
        fs::create_dir(&workspace).expect("workspace");
        fs::create_dir(&outside).expect("outside");
        fs::write(
            workspace.join("pom.xml"),
            pom(
                "<groupId>com.example</groupId><artifactId>parent</artifactId><version>1</version>
                 <packaging>pom</packaging><modules><module>../outside</module></modules>",
            ),
        )
        .expect("root POM");
        fs::write(
            outside.join("pom.xml"),
            pom(
                "<groupId>com.example</groupId><artifactId>outside</artifactId><version>1</version>",
            ),
        )
        .expect("outside POM");
        let repository = RepositoryClient::new(
            temporary.path().join("cache"),
            vec![format!(
                "file://{}",
                temporary.path().join("empty").display()
            )],
        )
        .expect("repository");

        let error = DependencyResolver::new(repository)
            .import_workspace(&workspace.join("pom.xml"))
            .await
            .expect_err("escaping module must fail");

        assert!(error.to_string().contains("escapes workspace"));
    }

    #[tokio::test]
    async fn resolves_unpublished_inter_module_dependencies_locally() {
        let temporary = tempfile::tempdir().expect("temporary directory");
        for module in ["library", "app"] {
            fs::create_dir(temporary.path().join(module)).expect("module directory");
        }
        fs::write(
            temporary.path().join("pom.xml"),
            pom(
                "<groupId>com.example</groupId><artifactId>parent</artifactId><version>1</version>
                 <packaging>pom</packaging><modules><module>library</module><module>app</module></modules>",
            ),
        )
        .expect("root POM");
        fs::write(
            temporary.path().join("library/pom.xml"),
            pom(
                "<parent><groupId>com.example</groupId><artifactId>parent</artifactId>
                 <version>1</version></parent><artifactId>library</artifactId>",
            ),
        )
        .expect("library POM");
        fs::write(
            temporary.path().join("app/pom.xml"),
            pom(
                "<parent><groupId>com.example</groupId><artifactId>parent</artifactId>
                 <version>1</version></parent><artifactId>app</artifactId>
                 <dependencies><dependency><groupId>com.example</groupId>
                 <artifactId>library</artifactId><version>1</version></dependency></dependencies>",
            ),
        )
        .expect("app POM");
        let repository = RepositoryClient::new(
            temporary.path().join("cache"),
            vec![format!(
                "file://{}",
                temporary.path().join("empty").display()
            )],
        )
        .expect("repository");
        let resolver = DependencyResolver::new(repository);
        let projects = resolver
            .import_workspace(&temporary.path().join("pom.xml"))
            .await
            .expect("reactor");
        let app = &projects[2].effective;

        let graph = resolver
            .resolve_in_workspace(app, &projects)
            .await
            .expect("local module resolution");

        assert_eq!(graph.packages.len(), 1);
        assert_eq!(graph.packages[0].coordinate.artifact, "library");
        assert!(graph.packages[0].artifact_checksum.is_none());
        assert!(graph.packages[0].source.starts_with("workspace:"));
    }

    #[tokio::test]
    async fn expands_the_mediated_version_when_a_later_path_adds_a_scope() {
        let temporary = tempfile::tempdir().expect("temporary directory");
        let repository_root = temporary.path().join("repository");
        for (version, child) in [("2", "selected-child"), ("1", "losing-child")] {
            write_artifact(
                &repository_root,
                "org.example",
                "library",
                version,
                "pom",
                &pom(&format!(
                    "<groupId>org.example</groupId><artifactId>library</artifactId>
                     <version>{version}</version><dependencies><dependency>
                     <groupId>org.example</groupId><artifactId>{child}</artifactId>
                     <version>1</version></dependency></dependencies>"
                )),
            );
            write_artifact(
                &repository_root,
                "org.example",
                "library",
                version,
                "jar",
                version,
            );
        }
        for child in ["selected-child", "losing-child"] {
            write_artifact(
                &repository_root,
                "org.example",
                child,
                "1",
                "pom",
                &pom(&format!(
                    "<groupId>org.example</groupId><artifactId>{child}</artifactId><version>1</version>"
                )),
            );
            write_artifact(&repository_root, "org.example", child, "1", "jar", child);
        }
        write_artifact(
            &repository_root,
            "org.example",
            "bridge",
            "1",
            "pom",
            &pom(
                "<groupId>org.example</groupId><artifactId>bridge</artifactId><version>1</version>
                 <dependencies><dependency><groupId>org.example</groupId>
                 <artifactId>library</artifactId><version>1</version></dependency></dependencies>",
            ),
        );
        write_artifact(
            &repository_root,
            "org.example",
            "bridge",
            "1",
            "jar",
            "bridge",
        );
        let root_pom = temporary.path().join("pom.xml");
        fs::write(
            &root_pom,
            pom(
                "<groupId>com.example</groupId><artifactId>app</artifactId><version>1</version>
                 <dependencies>
                 <dependency><groupId>org.example</groupId><artifactId>library</artifactId>
                 <version>2</version></dependency>
                 <dependency><groupId>org.example</groupId><artifactId>bridge</artifactId>
                 <version>1</version><scope>test</scope></dependency>
                 </dependencies>",
            ),
        )
        .expect("root POM");
        let repository = RepositoryClient::new(
            temporary.path().join("cache"),
            vec![format!("file://{}", repository_root.display())],
        )
        .expect("repository");
        let resolver = DependencyResolver::new(repository);
        let project = resolver
            .import_project(&root_pom)
            .await
            .expect("effective model");

        let graph = resolver.resolve(&project).await.expect("resolved graph");
        let artifacts = graph
            .packages
            .iter()
            .map(|package| package.coordinate.artifact.as_str())
            .collect::<Vec<_>>();

        assert!(artifacts.contains(&"selected-child"));
        assert!(!artifacts.contains(&"losing-child"));
    }

    #[tokio::test]
    async fn follows_maven_artifact_relocation() {
        let temporary = tempfile::tempdir().expect("temporary directory");
        let repository_root = temporary.path().join("repository");
        write_artifact(
            &repository_root,
            "org.legacy",
            "library",
            "1",
            "pom",
            &pom(
                "<groupId>org.legacy</groupId><artifactId>library</artifactId><version>1</version>
                 <distributionManagement><relocation><groupId>org.current</groupId>
                 <artifactId>library</artifactId><version>2</version>
                 </relocation></distributionManagement>",
            ),
        );
        write_artifact(
            &repository_root,
            "org.current",
            "library",
            "2",
            "pom",
            &pom(
                "<groupId>org.current</groupId><artifactId>library</artifactId><version>2</version>",
            ),
        );
        write_artifact(
            &repository_root,
            "org.current",
            "library",
            "2",
            "jar",
            "relocated",
        );
        let root_pom = temporary.path().join("pom.xml");
        fs::write(
            &root_pom,
            pom(
                "<groupId>com.example</groupId><artifactId>app</artifactId><version>1</version>
                 <dependencies><dependency><groupId>org.legacy</groupId>
                 <artifactId>library</artifactId><version>1</version></dependency></dependencies>",
            ),
        )
        .expect("root POM");
        let resolver = DependencyResolver::new(
            RepositoryClient::new(
                temporary.path().join("cache"),
                vec![format!("file://{}", repository_root.display())],
            )
            .expect("repository"),
        );
        let project = resolver
            .import_project(&root_pom)
            .await
            .expect("effective model");

        let graph = resolver.resolve(&project).await.expect("resolution");

        assert_eq!(graph.packages.len(), 1);
        assert_eq!(graph.packages[0].coordinate.group, "org.current");
        assert_eq!(graph.packages[0].coordinate.version, "2");
    }

    #[tokio::test]
    async fn direct_dependency_exclusions_prune_the_transitive_subtree() {
        let temporary = tempfile::tempdir().expect("temporary directory");
        let repository_root = temporary.path().join("repository");
        write_artifact(
            &repository_root,
            "org.example",
            "library",
            "1",
            "pom",
            &pom(
                "<groupId>org.example</groupId><artifactId>library</artifactId><version>1</version>
                 <dependencies><dependency><groupId>org.unwanted</groupId>
                 <artifactId>support</artifactId><version>1</version></dependency></dependencies>",
            ),
        );
        write_artifact(
            &repository_root,
            "org.example",
            "library",
            "1",
            "jar",
            "library",
        );
        let root_pom = temporary.path().join("pom.xml");
        fs::write(
            &root_pom,
            pom(
                "<groupId>com.example</groupId><artifactId>app</artifactId><version>1</version>
                 <dependencies><dependency><groupId>org.example</groupId>
                 <artifactId>library</artifactId><version>1</version><exclusions><exclusion>
                 <groupId>org.unwanted</groupId><artifactId>support</artifactId>
                 </exclusion></exclusions></dependency></dependencies>",
            ),
        )
        .expect("root POM");
        let resolver = DependencyResolver::new(
            RepositoryClient::new(
                temporary.path().join("cache"),
                vec![format!("file://{}", repository_root.display())],
            )
            .expect("repository"),
        );
        let project = resolver
            .import_project(&root_pom)
            .await
            .expect("effective model");

        let graph = resolver.resolve(&project).await.expect("resolution");

        assert_eq!(graph.packages.len(), 1);
        assert_eq!(graph.packages[0].coordinate.artifact, "library");
    }

    fn write_artifact(
        root: &Path,
        group: &str,
        artifact: &str,
        version: &str,
        extension: &str,
        contents: &str,
    ) {
        let directory = root
            .join(group.replace('.', "/"))
            .join(artifact)
            .join(version);
        fs::create_dir_all(&directory).expect("artifact directory");
        fs::write(
            directory.join(format!("{artifact}-{version}.{extension}")),
            contents,
        )
        .expect("artifact");
    }

    fn pom(body: &str) -> String {
        format!("<project><modelVersion>4.0.0</modelVersion>{body}</project>")
    }
}
