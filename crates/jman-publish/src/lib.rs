//! Build and publish JMAN workspaces using Maven repository conventions.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{Cursor, Write as _},
    path::{Path, PathBuf},
    process::Stdio,
    time::{Duration, Instant},
};

use anyhow::{anyhow, bail, Context, Result};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use jman_build::{ArtifactOptions, PackageResult};
use jman_config::{Manifest, MavenDependencyMetadata, Publishing};
use md5::Md5;
use reqwest::{multipart, Client, RequestBuilder, Url};
use serde::{Deserialize, Serialize};
use sha1::Sha1;
use sha2::{Digest, Sha256, Sha512};
use tokio::{io::AsyncWriteExt as _, process::Command, time::sleep};

/// Official Maven Central Publisher Portal API root.
pub const CENTRAL_API_URL: &str = "https://central.sonatype.com/api/v1/publisher/";

#[derive(Clone)]
pub enum PublishTarget {
    Local {
        repository: PathBuf,
    },
    Repository {
        url: Url,
        credentials: RepositoryCredentials,
        allow_insecure: bool,
    },
    Central {
        token: String,
        automatic: bool,
        api_url: Url,
    },
}

#[derive(Clone, Default)]
pub struct RepositoryCredentials {
    pub username: Option<String>,
    pub password: Option<String>,
    pub token: Option<String>,
}

#[derive(Clone)]
#[allow(clippy::struct_excessive_bools)]
pub struct PublishOptions {
    pub root: PathBuf,
    pub cache_dir: PathBuf,
    pub jobs: usize,
    pub offline: bool,
    pub dry_run: bool,
    pub sign: bool,
    pub gpg_key: Option<String>,
    pub gpg_passphrase: Option<String>,
    pub allow_dirty: bool,
    pub timeout: Duration,
    pub target: PublishTarget,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PublishReport {
    pub target: String,
    pub dry_run: bool,
    pub staging_directory: PathBuf,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bundle: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deployment_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deployment_state: Option<String>,
    pub modules: Vec<PublishedModule>,
    pub files: Vec<PublishedFile>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PublishedModule {
    pub group: String,
    pub artifact: String,
    pub version: String,
    pub packaging: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PublishedFile {
    pub path: String,
    pub size: u64,
    pub sha256: String,
    pub sha512: String,
}

#[derive(Clone, Debug)]
struct WorkspaceModule {
    directory: PathBuf,
    manifest: Manifest,
    dependencies: Vec<usize>,
    workspace_root: bool,
}

#[derive(Debug, Deserialize)]
struct CentralStatus {
    #[serde(rename = "deploymentState", alias = "state")]
    state: String,
    #[serde(default)]
    errors: serde_json::Value,
}

/// Build, stage, validate, and publish every workspace module.
///
/// # Errors
///
/// Returns an error for invalid publication metadata, build failures, signing
/// failures, unsafe repository targets, or rejected uploads.
pub async fn publish(options: &PublishOptions) -> Result<PublishReport> {
    if options.jobs == 0 {
        bail!("publish jobs must be at least 1");
    }
    let root = fs::canonicalize(&options.root)
        .with_context(|| format!("could not resolve {}", options.root.display()))?;
    let modules = discover_modules(&root)?;
    validate_workspace(&modules, &options.target)?;
    if let PublishTarget::Repository {
        url,
        credentials,
        allow_insecure,
    } = &options.target
    {
        validate_repository_url(url, *allow_insecure)?;
        validate_repository_credentials(credentials)?;
    }
    if is_remote(&options.target) && !options.dry_run && !options.allow_dirty {
        ensure_clean_worktree(&root).await?;
    }

    let packages = jman_build::package_workspace(
        &root,
        &options.cache_dir,
        options.jobs,
        options.offline,
        ArtifactOptions {
            fat: false,
            sources: true,
            javadoc: true,
        },
    )
    .await
    .context("could not build publication artifacts")?;

    let publication_root = root.join(".jman/publications");
    let staging = publication_root.join("repository");
    reset_directory(&staging)?;
    let primary_files = stage_modules(&staging, &modules, &packages)?;
    let must_sign = options.sign || matches!(options.target, PublishTarget::Central { .. });
    if must_sign {
        sign_files(
            &primary_files,
            options.gpg_key.as_deref(),
            options.gpg_passphrase.as_deref(),
        )
        .await?;
    }
    write_checksums(&primary_files)?;

    let files = describe_files(&staging)?;
    let mut report = PublishReport {
        target: target_name(&options.target).to_owned(),
        dry_run: options.dry_run,
        staging_directory: staging.clone(),
        bundle: None,
        deployment_id: None,
        deployment_state: None,
        modules: modules
            .iter()
            .map(|module| PublishedModule {
                group: module.manifest.project.group.clone(),
                artifact: module.manifest.project.name.clone(),
                version: module.manifest.project.version.clone(),
                packaging: module.manifest.project.packaging.clone(),
            })
            .collect(),
        files,
    };

    match &options.target {
        PublishTarget::Local { repository } => {
            if !options.dry_run {
                install_local(&staging, repository)?;
            }
        }
        PublishTarget::Repository {
            url,
            credentials,
            allow_insecure,
        } => {
            validate_repository_url(url, *allow_insecure)?;
            if !options.dry_run {
                upload_repository(&staging, url, credentials).await?;
            }
        }
        PublishTarget::Central {
            token,
            automatic,
            api_url,
        } => {
            let bundle = publication_root.join("central-bundle.zip");
            create_bundle(&staging, &bundle)?;
            report.bundle = Some(bundle.clone());
            if !options.dry_run {
                let (id, state) =
                    upload_central(&bundle, token, *automatic, api_url, options.timeout).await?;
                report.deployment_id = Some(id);
                report.deployment_state = Some(state);
            }
        }
    }
    Ok(report)
}

fn discover_modules(root: &Path) -> Result<Vec<WorkspaceModule>> {
    let mut pending = vec![root.to_owned()];
    let mut seen = BTreeSet::new();
    let mut raw = Vec::new();
    while let Some(directory) = pending.pop() {
        let directory = fs::canonicalize(&directory)
            .with_context(|| format!("could not resolve module {}", directory.display()))?;
        if !directory.starts_with(root) {
            bail!(
                "module {} escapes workspace {}",
                directory.display(),
                root.display()
            );
        }
        if !seen.insert(directory.clone()) {
            bail!(
                "duplicate or cyclic module declaration at {}",
                directory.display()
            );
        }
        let manifest = Manifest::read(&directory.join("jman.toml"))?;
        for child in manifest.project.modules.iter().rev() {
            pending.push(directory.join(child));
        }
        raw.push((directory, manifest));
    }
    let indices = raw
        .iter()
        .enumerate()
        .map(|(index, (directory, _))| (directory.clone(), index))
        .collect::<BTreeMap<_, _>>();
    let mut modules = Vec::with_capacity(raw.len());
    for (directory, manifest) in raw {
        let mut dependencies = Vec::new();
        for relative in manifest.path_dependencies.values() {
            let dependency = fs::canonicalize(directory.join(relative)).with_context(|| {
                format!(
                    "could not resolve path dependency {relative} from {}",
                    directory.display()
                )
            })?;
            dependencies.push(*indices.get(&dependency).ok_or_else(|| {
                anyhow!(
                    "path dependency {} from {} is not a declared workspace module",
                    dependency.display(),
                    directory.display()
                )
            })?);
        }
        dependencies.sort_unstable();
        dependencies.dedup();
        modules.push(WorkspaceModule {
            workspace_root: directory == root,
            directory,
            manifest,
            dependencies,
        });
    }
    let order = topological_order(&modules)?;
    Ok(order
        .into_iter()
        .map(|index| modules[index].clone())
        .collect())
}

fn topological_order(modules: &[WorkspaceModule]) -> Result<Vec<usize>> {
    let mut remaining = (0..modules.len()).collect::<BTreeSet<_>>();
    let mut complete = BTreeSet::new();
    let mut order = Vec::with_capacity(modules.len());
    while !remaining.is_empty() {
        let ready = remaining
            .iter()
            .copied()
            .filter(|index| {
                modules[*index]
                    .dependencies
                    .iter()
                    .all(|item| complete.contains(item))
            })
            .collect::<Vec<_>>();
        if ready.is_empty() {
            bail!("module dependency cycle prevents publication");
        }
        for index in ready {
            remaining.remove(&index);
            complete.insert(index);
            order.push(index);
        }
    }
    Ok(order)
}

fn validate_workspace(modules: &[WorkspaceModule], target: &PublishTarget) -> Result<()> {
    let mut names = BTreeSet::new();
    let root_publishing = modules
        .iter()
        .find(|module| module.workspace_root)
        .and_then(|module| module.manifest.publishing.as_ref());
    for module in modules {
        let project = &module.manifest.project;
        if !names.insert(project.name.as_str()) {
            bail!(
                "workspace module names must be unique for publication: {}",
                project.name
            );
        }
        validate_repository_segment(&project.name, "artifact name")?;
        validate_repository_segment(&project.version, "version")?;
        if !matches!(project.packaging.as_str(), "jar" | "pom") {
            bail!(
                "{} uses unsupported publication packaging `{}`; JMAN publishes jar and pom modules",
                project.name,
                project.packaging
            );
        }
        if matches!(target, PublishTarget::Central { .. }) {
            if project.version.ends_with("-SNAPSHOT") {
                bail!(
                    "Maven Central does not accept snapshot version {}",
                    project.version
                );
            }
            validate_central_metadata(
                module.manifest.publishing.as_ref().or(root_publishing),
                &project.name,
            )?;
        }
    }
    Ok(())
}

fn validate_repository_segment(value: &str, label: &str) -> Result<()> {
    if value.is_empty()
        || value == "."
        || value == ".."
        || !value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "-._+".contains(character))
    {
        bail!("publication {label} `{value}` is not a safe Maven repository path segment");
    }
    Ok(())
}

fn validate_central_metadata(metadata: Option<&Publishing>, module: &str) -> Result<()> {
    let metadata = metadata
        .ok_or_else(|| anyhow!("{module} requires [publishing] metadata for Maven Central"))?;
    for (name, value) in [
        ("name", metadata.name.as_deref()),
        ("description", metadata.description.as_deref()),
        ("url", metadata.url.as_deref()),
    ] {
        if value.is_none_or(|value| value.trim().is_empty()) {
            bail!("{module} requires publishing.{name} for Maven Central");
        }
    }
    if metadata.licenses.is_empty() {
        bail!("{module} requires at least one publishing license for Maven Central");
    }
    if metadata.developers.is_empty() {
        bail!("{module} requires at least one publishing developer for Maven Central");
    }
    if metadata.scm.is_none() {
        bail!("{module} requires publishing.scm for Maven Central");
    }
    validate_http_url(
        metadata.url.as_deref().expect("validated URL"),
        "project URL",
    )?;
    for license in &metadata.licenses {
        validate_http_url(&license.url, "license URL")?;
    }
    validate_http_url(
        &metadata.scm.as_ref().expect("validated SCM").url,
        "SCM URL",
    )?;
    Ok(())
}

fn validate_http_url(value: &str, name: &str) -> Result<()> {
    let url = Url::parse(value).with_context(|| format!("invalid publishing {name} `{value}`"))?;
    if !matches!(url.scheme(), "http" | "https") {
        bail!("publishing {name} must use HTTP or HTTPS");
    }
    Ok(())
}

async fn ensure_clean_worktree(root: &Path) -> Result<()> {
    let output = Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(root)
        .output()
        .await
        .context("could not inspect the Git worktree before publication")?;
    if !output.status.success() {
        bail!("could not inspect the Git worktree before publication");
    }
    if !output.stdout.is_empty() {
        bail!("refusing to publish a dirty Git worktree; commit changes or pass --allow-dirty");
    }
    Ok(())
}

fn stage_modules(
    staging: &Path,
    modules: &[WorkspaceModule],
    packages: &[PackageResult],
) -> Result<Vec<PathBuf>> {
    let by_module = packages.iter().fold(
        BTreeMap::<&str, BTreeMap<&str, &PackageResult>>::new(),
        |mut grouped, package| {
            grouped
                .entry(&package.module)
                .or_default()
                .insert(package.kind, package);
            grouped
        },
    );
    let root_publishing = modules
        .iter()
        .find(|module| module.workspace_root)
        .and_then(|module| module.manifest.publishing.as_ref());
    let mut primary = Vec::new();
    for module in modules {
        let project = &module.manifest.project;
        let directory = staging
            .join(project.group.replace('.', "/"))
            .join(&project.name)
            .join(&project.version);
        fs::create_dir_all(&directory)
            .with_context(|| format!("could not create {}", directory.display()))?;
        let prefix = format!("{}-{}", project.name, project.version);
        let pom = directory.join(format!("{prefix}.pom"));
        let xml = generate_pom(
            module,
            modules,
            module.manifest.publishing.as_ref().or(root_publishing),
        )?;
        fs::write(&pom, xml).with_context(|| format!("could not write {}", pom.display()))?;
        primary.push(pom);
        if project.packaging != "pom" {
            let artifacts = by_module.get(project.name.as_str()).ok_or_else(|| {
                anyhow!(
                    "build did not produce artifacts for module {}",
                    project.name
                )
            })?;
            for (kind, suffix) in [
                ("thin", ".jar"),
                ("sources", "-sources.jar"),
                ("javadoc", "-javadoc.jar"),
            ] {
                let package = artifacts.get(kind).ok_or_else(|| {
                    anyhow!("build did not produce {kind} artifact for {}", project.name)
                })?;
                let destination = directory.join(format!("{prefix}{suffix}"));
                fs::copy(&package.artifact, &destination).with_context(|| {
                    format!(
                        "could not stage {} as {}",
                        package.artifact.display(),
                        destination.display()
                    )
                })?;
                primary.push(destination);
            }
        }
    }
    primary.sort();
    Ok(primary)
}

fn generate_pom(
    module: &WorkspaceModule,
    modules: &[WorkspaceModule],
    publishing: Option<&Publishing>,
) -> Result<String> {
    let project = &module.manifest.project;
    let mut xml = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<project xmlns=\"http://maven.apache.org/POM/4.0.0\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" xsi:schemaLocation=\"http://maven.apache.org/POM/4.0.0 https://maven.apache.org/xsd/maven-4.0.0.xsd\">\n  <modelVersion>4.0.0</modelVersion>\n",
    );
    element(&mut xml, 2, "groupId", &project.group);
    element(&mut xml, 2, "artifactId", &project.name);
    element(&mut xml, 2, "version", &project.version);
    element(&mut xml, 2, "packaging", &project.packaging);
    element(
        &mut xml,
        2,
        "name",
        publishing
            .and_then(|value| value.name.as_deref())
            .unwrap_or(&project.name),
    );
    append_publishing_metadata(&mut xml, publishing);
    let dependencies = pom_dependencies(module, modules)?;
    if !dependencies.is_empty() {
        xml.push_str("  <dependencies>\n");
        for dependency in dependencies {
            xml.push_str("    <dependency>\n");
            element(&mut xml, 6, "groupId", &dependency.group);
            element(&mut xml, 6, "artifactId", &dependency.artifact);
            element(&mut xml, 6, "version", &dependency.version);
            if dependency.scope != "compile" {
                element(&mut xml, 6, "scope", dependency.scope);
            }
            if dependency.metadata.dependency_type != "jar" {
                element(&mut xml, 6, "type", &dependency.metadata.dependency_type);
            }
            optional_element(
                &mut xml,
                6,
                "classifier",
                dependency.metadata.classifier.as_deref(),
            );
            if dependency.metadata.optional {
                element(&mut xml, 6, "optional", "true");
            }
            if !dependency.metadata.exclusions.is_empty() {
                xml.push_str("      <exclusions>\n");
                for exclusion in &dependency.metadata.exclusions {
                    let (group, artifact) = split_ga(exclusion)?;
                    xml.push_str("        <exclusion>\n");
                    element(&mut xml, 10, "groupId", group);
                    element(&mut xml, 10, "artifactId", artifact);
                    xml.push_str("        </exclusion>\n");
                }
                xml.push_str("      </exclusions>\n");
            }
            xml.push_str("    </dependency>\n");
        }
        xml.push_str("  </dependencies>\n");
    }
    xml.push_str("</project>\n");
    Ok(xml)
}

fn append_publishing_metadata(xml: &mut String, publishing: Option<&Publishing>) {
    if let Some(metadata) = publishing {
        optional_element(xml, 2, "description", metadata.description.as_deref());
        optional_element(xml, 2, "url", metadata.url.as_deref());
        if !metadata.licenses.is_empty() {
            xml.push_str("  <licenses>\n");
            for license in &metadata.licenses {
                xml.push_str("    <license>\n");
                element(xml, 6, "name", &license.name);
                element(xml, 6, "url", &license.url);
                optional_element(xml, 6, "distribution", license.distribution.as_deref());
                xml.push_str("    </license>\n");
            }
            xml.push_str("  </licenses>\n");
        }
        if !metadata.developers.is_empty() {
            xml.push_str("  <developers>\n");
            for developer in &metadata.developers {
                xml.push_str("    <developer>\n");
                optional_element(xml, 6, "id", developer.id.as_deref());
                element(xml, 6, "name", &developer.name);
                optional_element(xml, 6, "email", developer.email.as_deref());
                optional_element(xml, 6, "organization", developer.organization.as_deref());
                optional_element(
                    xml,
                    6,
                    "organizationUrl",
                    developer.organization_url.as_deref(),
                );
                xml.push_str("    </developer>\n");
            }
            xml.push_str("  </developers>\n");
        }
        if let Some(scm) = &metadata.scm {
            xml.push_str("  <scm>\n");
            element(xml, 4, "connection", &scm.connection);
            element(xml, 4, "developerConnection", &scm.developer_connection);
            element(xml, 4, "url", &scm.url);
            optional_element(xml, 4, "tag", scm.tag.as_deref());
            xml.push_str("  </scm>\n");
        }
    }
}

struct PomDependency<'a> {
    group: String,
    artifact: String,
    version: String,
    scope: &'a str,
    metadata: MavenDependencyMetadata,
}

fn pom_dependencies<'a>(
    module: &'a WorkspaceModule,
    modules: &'a [WorkspaceModule],
) -> Result<Vec<PomDependency<'a>>> {
    let manifest = &module.manifest;
    let mut result = Vec::new();
    for (scope, dependencies) in [
        ("compile", &manifest.dependencies.compile),
        ("runtime", &manifest.dependencies.runtime),
        ("provided", &manifest.dependencies.provided),
        ("test", &manifest.dependencies.test),
    ] {
        for (coordinate, version) in dependencies {
            if version.trim().is_empty() {
                bail!("dependency {coordinate} has an empty publication version");
            }
            let (group, artifact) = split_ga(coordinate)?;
            let key = format!("{scope}:{coordinate}");
            let metadata = manifest
                .maven
                .as_ref()
                .and_then(|maven| maven.dependencies.get(&key))
                .cloned()
                .unwrap_or_default();
            result.push(PomDependency {
                group: group.to_owned(),
                artifact: artifact.to_owned(),
                version: version.clone(),
                scope,
                metadata,
            });
        }
    }
    for relative in manifest.path_dependencies.values() {
        let directory = fs::canonicalize(module.directory.join(relative))?;
        let dependency = modules
            .iter()
            .find(|candidate| candidate.directory == directory)
            .ok_or_else(|| anyhow!("undeclared workspace dependency {}", directory.display()))?;
        result.push(PomDependency {
            group: dependency.manifest.project.group.clone(),
            artifact: dependency.manifest.project.name.clone(),
            version: dependency.manifest.project.version.clone(),
            scope: "compile",
            metadata: MavenDependencyMetadata::default(),
        });
    }
    result.sort_by(|left, right| {
        (&left.scope, &left.group, &left.artifact).cmp(&(
            &right.scope,
            &right.group,
            &right.artifact,
        ))
    });
    Ok(result)
}

fn split_ga(coordinate: &str) -> Result<(&str, &str)> {
    coordinate
        .split_once(':')
        .ok_or_else(|| anyhow!("invalid Maven coordinate {coordinate}"))
}

fn element(xml: &mut String, indent: usize, name: &str, value: &str) {
    xml.push_str(&" ".repeat(indent));
    xml.push('<');
    xml.push_str(name);
    xml.push('>');
    xml.push_str(&escape_xml(value));
    xml.push_str("</");
    xml.push_str(name);
    xml.push_str(">\n");
}

fn optional_element(xml: &mut String, indent: usize, name: &str, value: Option<&str>) {
    if let Some(value) = value {
        element(xml, indent, name, value);
    }
}

fn escape_xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

async fn sign_files(files: &[PathBuf], key: Option<&str>, passphrase: Option<&str>) -> Result<()> {
    sign_files_with(Path::new("gpg"), files, key, passphrase).await
}

async fn sign_files_with(
    executable: &Path,
    files: &[PathBuf],
    key: Option<&str>,
    passphrase: Option<&str>,
) -> Result<()> {
    for file in files {
        let signature = signature_path(file);
        let mut command = gpg_command(executable, file, &signature, key, passphrase.is_some());
        let mut child = command
            .spawn()
            .with_context(|| format!("could not start GPG for {}", file.display()))?;
        if let (Some(value), Some(mut stdin)) = (passphrase, child.stdin.take()) {
            stdin.write_all(value.as_bytes()).await?;
            stdin.write_all(b"\n").await?;
        }
        let output = child.wait_with_output().await?;
        if !output.status.success() {
            bail!(
                "GPG signing failed for {}: {}",
                file.display(),
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
    }
    Ok(())
}

fn gpg_command(
    executable: &Path,
    file: &Path,
    signature: &Path,
    key: Option<&str>,
    passphrase: bool,
) -> Command {
    let mut command = Command::new(executable);
    command
        .args(["--batch", "--yes", "--armor", "--detach-sign"])
        .arg("--output")
        .arg(signature);
    if let Some(key) = key {
        command.arg("--local-user").arg(key);
    }
    if passphrase {
        command
            .args(["--pinentry-mode", "loopback", "--passphrase-fd", "0"])
            .stdin(Stdio::piped());
    }
    command
        .arg(file)
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    command
}

fn signature_path(file: &Path) -> PathBuf {
    PathBuf::from(format!("{}.asc", file.display()))
}

fn write_checksums(files: &[PathBuf]) -> Result<()> {
    for file in files {
        let bytes = fs::read(file)?;
        for (suffix, digest) in [
            ("md5", hex::encode(Md5::digest(&bytes))),
            ("sha1", hex::encode(Sha1::digest(&bytes))),
            ("sha256", hex::encode(Sha256::digest(&bytes))),
            ("sha512", hex::encode(Sha512::digest(&bytes))),
        ] {
            fs::write(format!("{}.{suffix}", file.display()), digest)?;
        }
    }
    Ok(())
}

fn describe_files(root: &Path) -> Result<Vec<PublishedFile>> {
    let mut paths = repository_files(root)?;
    paths.sort();
    paths
        .into_iter()
        .map(|path| {
            let bytes = fs::read(&path)?;
            Ok(PublishedFile {
                path: repository_path(root, &path)?,
                size: u64::try_from(bytes.len())?,
                sha256: hex::encode(Sha256::digest(&bytes)),
                sha512: hex::encode(Sha512::digest(&bytes)),
            })
        })
        .collect()
}

fn repository_files(root: &Path) -> Result<Vec<PathBuf>> {
    let mut pending = vec![root.to_owned()];
    let mut files = Vec::new();
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(&directory)? {
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                pending.push(entry.path());
            } else if entry.file_type()?.is_file() {
                files.push(entry.path());
            }
        }
    }
    Ok(files)
}

fn repository_path(root: &Path, file: &Path) -> Result<String> {
    Ok(file
        .strip_prefix(root)?
        .components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/"))
}

fn install_local(staging: &Path, repository: &Path) -> Result<()> {
    fs::create_dir_all(repository)?;
    for source in repository_files(staging)? {
        let relative = source.strip_prefix(staging)?;
        let destination = repository.join(relative);
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }
        let temporary = destination.with_extension(format!(
            "{}.jman-tmp-{}",
            destination
                .extension()
                .and_then(|value| value.to_str())
                .unwrap_or("file"),
            std::process::id()
        ));
        fs::copy(&source, &temporary)?;
        if destination.exists() {
            fs::remove_file(&destination)?;
        }
        fs::rename(&temporary, &destination)?;
    }
    Ok(())
}

fn validate_repository_url(url: &Url, allow_insecure: bool) -> Result<()> {
    if url.scheme() != "https" && !(allow_insecure && url.scheme() == "http") {
        bail!("repository URL must use HTTPS (use --allow-insecure only for local development)");
    }
    if !url.username().is_empty() || url.password().is_some() {
        bail!("repository credentials must come from environment variables, not the URL");
    }
    if url.query().is_some() || url.fragment().is_some() {
        bail!("repository URL cannot contain a query or fragment");
    }
    Ok(())
}

fn repository_base_url(url: &Url) -> Url {
    let mut base = url.clone();
    if !base.path().ends_with('/') {
        let path = format!("{}/", base.path());
        base.set_path(&path);
    }
    base
}

fn validate_repository_credentials(credentials: &RepositoryCredentials) -> Result<()> {
    if credentials.username.is_some() != credentials.password.is_some() {
        bail!("repository username and password must be provided together");
    }
    Ok(())
}

async fn upload_repository(
    staging: &Path,
    base: &Url,
    credentials: &RepositoryCredentials,
) -> Result<()> {
    validate_repository_credentials(credentials)?;
    let client = Client::new();
    let base = repository_base_url(base);
    let mut files = repository_files(staging)?;
    files.sort();
    for file in files {
        let relative = repository_path(staging, &file)?;
        let url = base
            .join(&relative)
            .with_context(|| format!("invalid repository path {relative}"))?;
        let request = authenticated(
            client
                .put(url)
                .header(reqwest::header::CONTENT_TYPE, "application/octet-stream")
                .body(fs::read(&file)?),
            credentials,
        );
        let response = request.send().await?;
        if !response.status().is_success() {
            let status = response.status();
            let message = response.text().await.unwrap_or_default();
            bail!("repository rejected {relative} with {status}: {message}");
        }
    }
    Ok(())
}

fn authenticated(request: RequestBuilder, credentials: &RepositoryCredentials) -> RequestBuilder {
    if let Some(token) = &credentials.token {
        request.bearer_auth(token)
    } else if let (Some(username), Some(password)) = (&credentials.username, &credentials.password)
    {
        request.basic_auth(username, Some(password))
    } else {
        request
    }
}

fn create_bundle(staging: &Path, bundle: &Path) -> Result<()> {
    let mut files = repository_files(staging)?;
    files.sort();
    let mut archive = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .last_modified_time(zip::DateTime::default())
        .unix_permissions(0o644);
    for file in files {
        archive.start_file(repository_path(staging, &file)?, options)?;
        archive.write_all(&fs::read(file)?)?;
    }
    let bytes = archive.finish()?.into_inner();
    if let Some(parent) = bundle.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(bundle, bytes)?;
    Ok(())
}

async fn upload_central(
    bundle: &Path,
    token: &str,
    automatic: bool,
    api_url: &Url,
    timeout: Duration,
) -> Result<(String, String)> {
    if token.trim().is_empty() {
        bail!("Maven Central token cannot be empty");
    }
    let client = Client::new();
    let upload_url = api_url.join("upload")?;
    let file_name = bundle
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("central-bundle.zip")
        .to_owned();
    let part = multipart::Part::bytes(fs::read(bundle)?)
        .file_name(file_name)
        .mime_str("application/zip")?;
    let response = client
        .post(upload_url)
        .bearer_auth(token)
        .query(&[
            ("name", "JMAN publication"),
            (
                "publishingType",
                if automatic {
                    "AUTOMATIC"
                } else {
                    "USER_MANAGED"
                },
            ),
        ])
        .multipart(multipart::Form::new().part("bundle", part))
        .send()
        .await?;
    if !response.status().is_success() {
        let status = response.status();
        let message = response.text().await.unwrap_or_default();
        bail!("Maven Central rejected the bundle with {status}: {message}");
    }
    let id = response.text().await?.trim_matches('"').trim().to_owned();
    if id.is_empty() {
        bail!("Maven Central returned an empty deployment ID");
    }
    let terminal = if automatic { "PUBLISHED" } else { "VALIDATED" };
    let started = Instant::now();
    loop {
        if started.elapsed() >= timeout {
            bail!("timed out waiting for Maven Central deployment {id}");
        }
        let response = client
            .post(api_url.join("status")?)
            .bearer_auth(token)
            .query(&[("id", &id)])
            .send()
            .await?;
        if !response.status().is_success() {
            let status = response.status();
            let message = response.text().await.unwrap_or_default();
            bail!("could not read Maven Central deployment {id}: {status}: {message}");
        }
        let status: CentralStatus = response.json().await?;
        if status.state == terminal {
            return Ok((id, status.state));
        }
        if status.state == "FAILED" {
            bail!(
                "Maven Central deployment {id} failed validation: {}",
                status.errors
            );
        }
        sleep(Duration::from_secs(2)).await;
    }
}

fn reset_directory(path: &Path) -> Result<()> {
    if path.exists() {
        fs::remove_dir_all(path).with_context(|| format!("could not reset {}", path.display()))?;
    }
    fs::create_dir_all(path).with_context(|| format!("could not create {}", path.display()))?;
    Ok(())
}

fn is_remote(target: &PublishTarget) -> bool {
    matches!(
        target,
        PublishTarget::Repository { .. } | PublishTarget::Central { .. }
    )
}

fn target_name(target: &PublishTarget) -> &'static str {
    match target {
        PublishTarget::Local { .. } => "local",
        PublishTarget::Repository { .. } => "repository",
        PublishTarget::Central { .. } => "central",
    }
}

/// Build the Central bearer value from a portal username/password pair.
#[must_use]
pub fn central_token(username: &str, password: &str) -> String {
    BASE64.encode(format!("{username}:{password}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use jman_config::{
        Dependencies, Developer, License, MavenCompatibility, Project, Scm, MANIFEST_VERSION,
    };
    use std::{
        io::Read as _,
        net::{TcpListener, TcpStream},
        thread,
    };

    fn manifest(name: &str) -> Manifest {
        Manifest {
            manifest_version: MANIFEST_VERSION,
            project: Project {
                group: "com.example".to_owned(),
                name: name.to_owned(),
                version: "1.2.3".to_owned(),
                java_release: 21,
                packaging: "jar".to_owned(),
                modules: Vec::new(),
                main_class: None,
            },
            toolchain: None,
            maven: None,
            build: None,
            publishing: Some(Publishing {
                name: Some("Demo & API".to_owned()),
                description: Some("Fast <Java> library".to_owned()),
                url: Some("https://example.test/demo".to_owned()),
                licenses: vec![License {
                    name: "Apache-2.0".to_owned(),
                    url: "https://www.apache.org/licenses/LICENSE-2.0.txt".to_owned(),
                    distribution: Some("repo".to_owned()),
                }],
                developers: vec![Developer {
                    id: Some("dev".to_owned()),
                    name: "Developer".to_owned(),
                    email: Some("dev@example.test".to_owned()),
                    organization: None,
                    organization_url: None,
                }],
                scm: Some(Scm {
                    connection: "scm:git:https://example.test/demo.git".to_owned(),
                    developer_connection: "scm:git:ssh://example.test/demo.git".to_owned(),
                    url: "https://example.test/demo".to_owned(),
                    tag: Some("HEAD".to_owned()),
                }),
            }),
            repositories: Vec::new(),
            dependencies: Dependencies::default(),
            annotation_processors: BTreeMap::new(),
            path_dependencies: BTreeMap::new(),
        }
    }

    fn module(directory: &Path, name: &str) -> WorkspaceModule {
        WorkspaceModule {
            directory: directory.to_owned(),
            manifest: manifest(name),
            dependencies: Vec::new(),
            workspace_root: false,
        }
    }

    #[test]
    fn generated_pom_contains_metadata_and_escapes_xml() {
        let directory = PathBuf::from("/tmp/demo");
        let module = module(&directory, "demo");
        let xml = generate_pom(
            &module,
            std::slice::from_ref(&module),
            module.manifest.publishing.as_ref(),
        )
        .unwrap();
        assert!(xml.contains("<name>Demo &amp; API</name>"));
        assert!(xml.contains("<description>Fast &lt;Java&gt; library</description>"));
        assert!(xml.contains("<developerConnection>scm:git:ssh://example.test/demo.git"));
    }

    #[test]
    fn generated_pom_maps_scopes_metadata_and_workspace_dependencies() {
        let temporary = tempfile::tempdir().unwrap();
        let library_directory = temporary.path().join("api");
        let consumer_directory = temporary.path().join("app");
        fs::create_dir_all(&library_directory).unwrap();
        fs::create_dir_all(&consumer_directory).unwrap();
        let api = module(&library_directory.canonicalize().unwrap(), "api");
        let mut app = module(&consumer_directory.canonicalize().unwrap(), "app");
        app.manifest
            .dependencies
            .runtime
            .insert("org.example:runtime".to_owned(), "4.0".to_owned());
        app.manifest.maven = Some(MavenCompatibility {
            parent: None,
            dependencies: BTreeMap::from([(
                "runtime:org.example:runtime".to_owned(),
                MavenDependencyMetadata {
                    dependency_type: "test-jar".to_owned(),
                    classifier: Some("tests".to_owned()),
                    optional: true,
                    exclusions: vec!["org.unwanted:legacy".to_owned()],
                },
            )]),
            dependency_management: Vec::new(),
            bom_imports: Vec::new(),
        });
        app.manifest
            .path_dependencies
            .insert("com.example:api".to_owned(), "../api".to_owned());
        let xml = generate_pom(&app, &[api, app.clone()], None).unwrap();
        assert!(xml.contains("<artifactId>api</artifactId>"));
        assert!(xml.contains("<version>1.2.3</version>"));
        assert!(xml.contains("<artifactId>runtime</artifactId>"));
        assert!(xml.contains("<scope>runtime</scope>"));
        assert!(xml.contains("<type>test-jar</type>"));
        assert!(xml.contains("<classifier>tests</classifier>"));
        assert!(xml.contains("<optional>true</optional>"));
        assert!(xml.contains("<artifactId>legacy</artifactId>"));
    }

    #[test]
    fn central_requires_release_version_and_complete_metadata() {
        let directory = PathBuf::from("/tmp/demo");
        let mut module = module(&directory, "demo");
        module.manifest.project.version = "1.0-SNAPSHOT".to_owned();
        let target = PublishTarget::Central {
            token: "token".to_owned(),
            automatic: false,
            api_url: Url::parse(CENTRAL_API_URL).unwrap(),
        };
        assert!(validate_workspace(&[module.clone()], &target)
            .unwrap_err()
            .to_string()
            .contains("snapshot"));
        module.manifest.project.version = "1.0".to_owned();
        module.manifest.publishing = None;
        assert!(validate_workspace(&[module], &target)
            .unwrap_err()
            .to_string()
            .contains("[publishing]"));
    }

    #[test]
    fn publication_coordinates_cannot_escape_the_repository_layout() {
        let directory = PathBuf::from("/tmp/demo");
        let mut invalid_name = module(&directory, "../outside");
        let local = PublishTarget::Local {
            repository: directory.clone(),
        };
        assert!(validate_workspace(&[invalid_name.clone()], &local).is_err());
        invalid_name.manifest.project.name = "demo".to_owned();
        invalid_name.manifest.project.version = "../../outside".to_owned();
        assert!(validate_workspace(&[invalid_name], &local).is_err());
    }

    #[test]
    fn checksums_match_primary_file_and_are_not_recursive() {
        let temporary = tempfile::tempdir().unwrap();
        let file = temporary.path().join("demo.jar");
        fs::write(&file, b"hello").unwrap();
        write_checksums(std::slice::from_ref(&file)).unwrap();
        assert_eq!(
            fs::read_to_string(file.with_extension("jar.sha256")).unwrap(),
            hex::encode(Sha256::digest(b"hello"))
        );
        assert!(!file.with_extension("jar.sha256.sha256").exists());
    }

    #[test]
    fn central_token_uses_portal_basic_credentials_payload() {
        assert_eq!(central_token("user", "secret"), "dXNlcjpzZWNyZXQ=");
    }

    #[test]
    fn repository_url_rejects_credentials_and_plain_http() {
        assert!(validate_repository_url(&Url::parse("http://repo.test/").unwrap(), false).is_err());
        assert!(validate_repository_url(&Url::parse("http://127.0.0.1/").unwrap(), true).is_ok());
        assert!(validate_repository_url(
            &Url::parse("https://user:secret@repo.test/").unwrap(),
            false
        )
        .is_err());
        assert!(validate_repository_url(
            &Url::parse("https://repo.test/releases?credential=secret").unwrap(),
            false
        )
        .is_err());
    }

    #[test]
    fn repository_credentials_require_a_complete_basic_auth_pair() {
        assert!(validate_repository_credentials(&RepositoryCredentials {
            username: Some("user".to_owned()),
            ..RepositoryCredentials::default()
        })
        .is_err());
        assert!(validate_repository_credentials(&RepositoryCredentials {
            username: Some("user".to_owned()),
            password: Some("password".to_owned()),
            token: None,
        })
        .is_ok());
    }

    #[test]
    fn gpg_passphrase_is_supplied_through_standard_input_not_arguments() {
        let command = gpg_command(
            Path::new("gpg"),
            Path::new("artifact.jar"),
            Path::new("artifact.jar.asc"),
            Some("ABC123"),
            true,
        );
        let arguments = command
            .as_std()
            .get_args()
            .map(|value| value.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert!(arguments
            .windows(2)
            .any(|values| values == ["--passphrase-fd", "0"]));
        assert!(!arguments.iter().any(|value| value == "--passphrase"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn signing_writes_detached_signature_for_every_primary_file() {
        use std::os::unix::fs::PermissionsExt as _;

        let temporary = tempfile::tempdir().unwrap();
        let signer = temporary.path().join("fake-gpg");
        fs::write(
            &signer,
            "#!/bin/sh\noutput=''\nwhile [ \"$#\" -gt 0 ]; do\n  if [ \"$1\" = '--output' ]; then output=\"$2\"; shift 2; else shift; fi\ndone\ncat >/dev/null\nprintf signature > \"$output\"\n",
        )
        .unwrap();
        fs::set_permissions(&signer, fs::Permissions::from_mode(0o755)).unwrap();
        let pom = temporary.path().join("demo.pom");
        let jar = temporary.path().join("demo.jar");
        fs::write(&pom, b"pom").unwrap();
        fs::write(&jar, b"jar").unwrap();
        sign_files_with(
            &signer,
            &[pom.clone(), jar.clone()],
            Some("ABC123"),
            Some("secret"),
        )
        .await
        .unwrap();
        assert_eq!(fs::read(signature_path(&pom)).unwrap(), b"signature");
        assert_eq!(fs::read(signature_path(&jar)).unwrap(), b"signature");
    }

    #[test]
    fn local_install_preserves_repository_layout() {
        let temporary = tempfile::tempdir().unwrap();
        let staging = temporary.path().join("stage");
        let repository = temporary.path().join("repository");
        let source = staging.join("com/example/demo/1/demo-1.pom");
        fs::create_dir_all(source.parent().unwrap()).unwrap();
        fs::write(&source, b"pom").unwrap();
        install_local(&staging, &repository).unwrap();
        assert_eq!(
            fs::read(repository.join("com/example/demo/1/demo-1.pom")).unwrap(),
            b"pom"
        );
    }

    #[test]
    fn central_bundle_is_reproducible_and_uses_forward_slashes() {
        let temporary = tempfile::tempdir().unwrap();
        let staging = temporary.path().join("stage");
        let file = staging.join("com/example/demo/1/demo-1.pom");
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(&file, b"pom").unwrap();
        let first = temporary.path().join("first.zip");
        let second = temporary.path().join("second.zip");
        create_bundle(&staging, &first).unwrap();
        create_bundle(&staging, &second).unwrap();
        assert_eq!(fs::read(&first).unwrap(), fs::read(&second).unwrap());
        let mut archive = zip::ZipArchive::new(fs::File::open(first).unwrap()).unwrap();
        assert_eq!(
            archive.by_index(0).unwrap().name(),
            "com/example/demo/1/demo-1.pom"
        );
    }

    #[test]
    fn topological_order_places_workspace_dependencies_first_and_rejects_cycles() {
        let directory = PathBuf::from("/tmp/workspace");
        let mut application = module(&directory.join("application"), "application");
        application.dependencies = vec![1];
        let library = module(&directory.join("library"), "library");
        assert_eq!(
            topological_order(&[application.clone(), library.clone()]).unwrap(),
            vec![1, 0]
        );

        let mut cyclic_library = library;
        cyclic_library.dependencies = vec![0];
        assert!(topological_order(&[application, cyclic_library]).is_err());
    }

    #[tokio::test]
    async fn generic_repository_uses_maven_path_put_and_bearer_authentication() {
        let temporary = tempfile::tempdir().unwrap();
        let staging = temporary.path().join("stage");
        let file = staging.join("com/example/demo/1/demo-1.pom");
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(&file, b"pom").unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let request = read_request(&mut stream);
            stream
                .write_all(b"HTTP/1.1 201 Created\r\nContent-Length: 0\r\n\r\n")
                .unwrap();
            request
        });
        let url = Url::parse(&format!("http://{address}/releases")).unwrap();
        upload_repository(
            &staging,
            &url,
            &RepositoryCredentials {
                token: Some("secret-token".to_owned()),
                ..RepositoryCredentials::default()
            },
        )
        .await
        .unwrap();
        let request = server.join().unwrap();
        assert!(request.starts_with("PUT /releases/com/example/demo/1/demo-1.pom HTTP/1.1"));
        assert!(request
            .to_ascii_lowercase()
            .contains("authorization: bearer secret-token"));
        assert!(request.to_ascii_lowercase().contains("content-length: 3"));
    }

    #[tokio::test]
    async fn central_upload_uses_bundle_endpoint_and_waits_for_validation() {
        let temporary = tempfile::tempdir().unwrap();
        let bundle = temporary.path().join("bundle.zip");
        fs::write(&bundle, b"zip bytes").unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut upload, _) = listener.accept().unwrap();
            let upload_request = read_request(&mut upload);
            upload
                .write_all(
                    b"HTTP/1.1 201 Created\r\nContent-Length: 12\r\nConnection: close\r\n\r\ndeployment-1",
                )
                .unwrap();
            let (mut status, _) = listener.accept().unwrap();
            let status_request = read_request(&mut status);
            let body = b"{\"deploymentState\":\"VALIDATED\"}";
            status
                .write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",
                        body.len()
                    )
                    .as_bytes(),
                )
                .unwrap();
            status.write_all(body).unwrap();
            (upload_request, status_request)
        });
        let api_url = Url::parse(&format!("http://{address}/api/v1/publisher/")).unwrap();
        let result = upload_central(
            &bundle,
            "encoded-token",
            false,
            &api_url,
            Duration::from_secs(2),
        )
        .await
        .unwrap();
        assert_eq!(result, ("deployment-1".to_owned(), "VALIDATED".to_owned()));
        let (upload, status) = server.join().unwrap();
        assert!(upload.starts_with("POST /api/v1/publisher/upload?"));
        assert!(upload.contains("publishingType=USER_MANAGED"));
        assert!(upload
            .to_ascii_lowercase()
            .contains("authorization: bearer encoded-token"));
        assert!(upload
            .to_ascii_lowercase()
            .contains("content-type: multipart/form-data"));
        assert!(status.starts_with("POST /api/v1/publisher/status?id=deployment-1"));
    }

    fn read_request(stream: &mut TcpStream) -> String {
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut bytes = Vec::new();
        let mut buffer = [0_u8; 4096];
        loop {
            let read = stream.read(&mut buffer).unwrap();
            if read == 0 {
                break;
            }
            bytes.extend_from_slice(&buffer[..read]);
            if bytes.windows(4).any(|window| window == b"\r\n\r\n") {
                break;
            }
        }
        String::from_utf8_lossy(&bytes).into_owned()
    }
}
