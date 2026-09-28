//! Project manifest and lockfile models.

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Current public manifest schema version.
pub const MANIFEST_VERSION: u32 = 1;

/// Current public lockfile schema version.
pub const LOCK_VERSION: u32 = 3;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("could not read {path}: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("could not parse {path}: {source}")]
    Parse {
        path: PathBuf,
        source: toml::de::Error,
    },
    #[error("could not serialize configuration: {0}")]
    Serialize(#[from] toml::ser::Error),
    #[error("unsupported {kind} schema version {actual}; this jman supports version {supported}")]
    UnsupportedVersion {
        kind: &'static str,
        actual: u32,
        supported: u32,
    },
    #[error(
        "{path} selects a Java toolchain but does not define a JMAN project; add [project] or run `jman init`"
    )]
    ToolchainOnly { path: PathBuf },
    #[error("invalid configuration: {0}")]
    Validation(String),
    #[error("invalid Java selection in {path}: {message}")]
    JavaSelection { path: PathBuf, message: String },
}

/// A parsed `jman.toml`, which may select only Java or define a full project.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ManifestFile {
    Toolchain(ToolchainManifest),
    Project(Box<Manifest>),
}

/// Origin of a project-local Java selection.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum JavaSelectionSource {
    JmanProject,
    JmanDirectory,
    Sdkmanrc,
    JavaVersion,
}

impl JavaSelectionSource {
    /// Human-readable source label used by diagnostics.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::JmanProject => "JMAN project",
            Self::JmanDirectory => "JMAN directory",
            Self::Sdkmanrc => ".sdkmanrc",
            Self::JavaVersion => ".java-version",
        }
    }
}

/// Project-local Java request discovered from a supported version file.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JavaSelection {
    pub path: PathBuf,
    pub version: String,
    pub vendor: Option<String>,
    pub source: JavaSelectionSource,
}

/// A `jman.toml` that selects Java without declaring a native JMAN project.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct ToolchainManifest {
    pub manifest_version: u32,
    pub toolchain: Toolchain,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub struct Manifest {
    pub manifest_version: u32,
    pub project: Project,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub toolchain: Option<Toolchain>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub maven: Option<MavenCompatibility>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub build: Option<Build>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub test: Option<Test>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub publishing: Option<Publishing>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audit: Option<Audit>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub repositories: Vec<Repository>,
    #[serde(default, skip_serializing_if = "Dependencies::is_empty")]
    pub dependencies: Dependencies,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub annotation_processors: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub path_dependencies: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Toolchain {
    /// JDK major (`17`) or exact version (`17.0.20+8`).
    pub jdk: String,
    #[serde(default = "default_jdk_vendor")]
    pub vendor: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct MavenCompatibility {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    /// Non-default Maven metadata for direct dependencies, keyed by
    /// `scope:group:artifact`.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub dependencies: BTreeMap<String, MavenDependencyMetadata>,
    /// Effective dependency-management entries retained after import.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dependency_management: Vec<MavenManagedDependency>,
    /// Imported BOM coordinates retained as provenance.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bom_imports: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub struct MavenDependencyMetadata {
    #[serde(default = "default_dependency_type", skip_serializing_if = "is_jar")]
    pub dependency_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub classifier: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub optional: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exclusions: Vec<String>,
}

impl Default for MavenDependencyMetadata {
    fn default() -> Self {
        Self {
            dependency_type: default_dependency_type(),
            classifier: None,
            optional: false,
            exclusions: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub struct MavenManagedDependency {
    pub group: String,
    pub artifact: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default = "default_compile_scope")]
    pub scope: String,
    #[serde(default = "default_dependency_type")]
    pub dependency_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub classifier: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub optional: Option<bool>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exclusions: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub struct Project {
    pub group: String,
    pub name: String,
    pub version: String,
    pub java_release: u16,
    #[serde(default = "default_packaging")]
    pub packaging: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub modules: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub main_class: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub struct Build {
    #[serde(default = "default_encoding")]
    pub encoding: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub compiler_args: Vec<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub struct Test {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coverage: Option<Coverage>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub struct Coverage {
    #[serde(default, skip_serializing_if = "is_false")]
    pub enabled: bool,
    #[serde(default = "default_coverage_engine", skip_serializing_if = "is_jacoco")]
    pub engine: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub formats: Vec<CoverageFormat>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minimum_line: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minimum_branch: Option<u8>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub include: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exclude: Vec<String>,
}

impl Default for Coverage {
    fn default() -> Self {
        Self {
            enabled: false,
            engine: default_coverage_engine(),
            formats: Vec::new(),
            minimum_line: None,
            minimum_branch: None,
            include: Vec::new(),
            exclude: Vec::new(),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CoverageFormat {
    Summary,
    Json,
    Xml,
    Html,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub struct Publishing {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub licenses: Vec<License>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub developers: Vec<Developer>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scm: Option<Scm>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct License {
    pub name: String,
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub distribution: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub struct Developer {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub organization: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub organization_url: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub struct Scm {
    pub connection: String,
    pub developer_connection: String,
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct Audit {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub suppressions: Vec<AuditSuppression>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AuditSuppression {
    pub id: String,
    pub reason: String,
    pub expires: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Repository {
    pub id: String,
    pub url: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub struct Lockfile {
    pub lock_version: u32,
    pub manifest_hash: String,
    pub workspace_hash: String,
    pub platform: String,
    pub toolchain: LockedToolchain,
    #[serde(rename = "package", default)]
    pub packages: Vec<LockedPackage>,
    pub classpath: LockedClasspaths,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub struct LockedToolchain {
    pub java_release: u16,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct LockedPackage {
    pub group: String,
    pub artifact: String,
    pub version: String,
    pub extension: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub classifier: Option<String>,
    pub source: String,
    pub pom_checksum: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact_checksum: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact_size: Option<u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dependencies: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scopes: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected_parent: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct LockedClasspaths {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub compile: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub runtime: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub test: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub processors: Vec<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub struct Dependencies {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub compile: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub runtime: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub provided: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub test: BTreeMap<String, String>,
}

impl Dependencies {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.compile.is_empty()
            && self.runtime.is_empty()
            && self.provided.is_empty()
            && self.test.is_empty()
    }
}

impl Manifest {
    /// Parse and validate a manifest.
    ///
    /// # Errors
    ///
    /// Returns an error when the file cannot be read, parsed, or validated.
    pub fn read(path: &Path) -> Result<Self, ConfigError> {
        match ManifestFile::read(path)? {
            ManifestFile::Project(manifest) => Ok(*manifest),
            ManifestFile::Toolchain(_) => Err(ConfigError::ToolchainOnly {
                path: path.to_owned(),
            }),
        }
    }

    /// Validate schema compatibility and semantic invariants.
    ///
    /// # Errors
    ///
    /// Returns an error for unsupported schemas, names, or coordinates.
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.manifest_version != MANIFEST_VERSION {
            return Err(ConfigError::UnsupportedVersion {
                kind: "manifest",
                actual: self.manifest_version,
                supported: MANIFEST_VERSION,
            });
        }
        validate_java_name("project group", &self.project.group)?;
        if self.project.name.trim().is_empty() {
            return Err(ConfigError::Validation(
                "project name cannot be empty".to_owned(),
            ));
        }
        if let Some(main_class) = &self.project.main_class {
            validate_java_name("main class", main_class)?;
        }
        if let Some(toolchain) = &self.toolchain {
            let major = java_feature_version(&toolchain.jdk);
            if major.is_none_or(|major| major < self.project.java_release) {
                return Err(ConfigError::Validation(format!(
                    "toolchain JDK `{}` must be a valid version at least as new as Java release {}",
                    toolchain.jdk, self.project.java_release
                )));
            }
            validate_jdk_vendor(&toolchain.vendor)?;
        }
        for coordinate in self
            .dependencies
            .compile
            .keys()
            .chain(self.dependencies.runtime.keys())
            .chain(self.dependencies.provided.keys())
            .chain(self.dependencies.test.keys())
            .chain(self.annotation_processors.keys())
        {
            validate_ga(coordinate)?;
        }
        if let Some(maven) = &self.maven {
            for (key, metadata) in &maven.dependencies {
                let (scope, coordinate) = key.split_once(':').ok_or_else(|| {
                    ConfigError::Validation(format!(
                        "Maven dependency metadata key `{key}` must use scope:group:artifact"
                    ))
                })?;
                if !matches!(scope, "compile" | "runtime" | "provided" | "test") {
                    return Err(ConfigError::Validation(format!(
                        "unsupported dependency metadata scope `{scope}`"
                    )));
                }
                validate_ga(coordinate)?;
                for exclusion in &metadata.exclusions {
                    validate_ga(exclusion)?;
                }
            }
            for managed in &maven.dependency_management {
                validate_ga(&format!("{}:{}", managed.group, managed.artifact))?;
                for exclusion in &managed.exclusions {
                    validate_ga(exclusion)?;
                }
            }
        }
        if let Some(publishing) = &self.publishing {
            validate_publishing(publishing)?;
        }
        if let Some(audit) = &self.audit {
            validate_audit(audit)?;
        }
        if let Some(coverage) = self.test.as_ref().and_then(|test| test.coverage.as_ref()) {
            validate_coverage(coverage)?;
        }
        Ok(())
    }

    /// Serialize a validated manifest deterministically.
    ///
    /// # Errors
    ///
    /// Returns an error when validation or TOML serialization fails.
    pub fn to_toml(&self) -> Result<String, ConfigError> {
        self.validate()?;
        Ok(toml::to_string_pretty(self)?)
    }
}

impl ManifestFile {
    /// Parse and validate either supported `jman.toml` mode.
    ///
    /// # Errors
    ///
    /// Returns an error when the file cannot be read, parsed, or validated.
    pub fn read(path: &Path) -> Result<Self, ConfigError> {
        #[derive(Deserialize)]
        struct ManifestProbe {
            project: Option<toml::Value>,
        }

        let text = fs::read_to_string(path).map_err(|source| ConfigError::Read {
            path: path.to_owned(),
            source,
        })?;
        let probe: ManifestProbe = toml::from_str(&text).map_err(|source| ConfigError::Parse {
            path: path.to_owned(),
            source,
        })?;
        if probe.project.is_some() {
            let manifest: Manifest =
                toml::from_str(&text).map_err(|source| ConfigError::Parse {
                    path: path.to_owned(),
                    source,
                })?;
            manifest.validate()?;
            return Ok(Self::Project(Box::new(manifest)));
        }

        let manifest: ToolchainManifest =
            toml::from_str(&text).map_err(|source| ConfigError::Parse {
                path: path.to_owned(),
                source,
            })?;
        manifest.validate()?;
        Ok(Self::Toolchain(manifest))
    }

    #[must_use]
    pub const fn toolchain(&self) -> Option<&Toolchain> {
        match self {
            Self::Toolchain(manifest) => Some(&manifest.toolchain),
            Self::Project(manifest) => manifest.toolchain.as_ref(),
        }
    }

    #[must_use]
    pub const fn is_project(&self) -> bool {
        matches!(self, Self::Project(_))
    }
}

impl ToolchainManifest {
    /// Validate schema compatibility and the selected JDK identity.
    ///
    /// # Errors
    ///
    /// Returns an error for an unsupported schema or invalid selection.
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.manifest_version != MANIFEST_VERSION {
            return Err(ConfigError::UnsupportedVersion {
                kind: "manifest",
                actual: self.manifest_version,
                supported: MANIFEST_VERSION,
            });
        }
        if java_feature_version(&self.toolchain.jdk).is_none() {
            return Err(ConfigError::Validation(format!(
                "toolchain JDK `{}` must be a valid Java version",
                self.toolchain.jdk
            )));
        }
        validate_jdk_vendor(&self.toolchain.vendor)
    }

    /// Serialize a validated toolchain-only manifest deterministically.
    ///
    /// # Errors
    ///
    /// Returns an error when validation or TOML serialization fails.
    pub fn to_toml(&self) -> Result<String, ConfigError> {
        self.validate()?;
        Ok(toml::to_string_pretty(self)?)
    }
}

impl Lockfile {
    /// Parse and validate a lockfile.
    ///
    /// # Errors
    ///
    /// Returns an error when the file cannot be read, parsed, or validated.
    pub fn read(path: &Path) -> Result<Self, ConfigError> {
        let text = fs::read_to_string(path).map_err(|source| ConfigError::Read {
            path: path.to_owned(),
            source,
        })?;
        let lock: Self = toml::from_str(&text).map_err(|source| ConfigError::Parse {
            path: path.to_owned(),
            source,
        })?;
        lock.validate()?;
        Ok(lock)
    }

    /// Validate the lockfile schema and integrity field shapes.
    ///
    /// # Errors
    ///
    /// Returns an error for unsupported schemas or malformed hashes.
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.lock_version != LOCK_VERSION {
            return Err(ConfigError::UnsupportedVersion {
                kind: "lockfile",
                actual: self.lock_version,
                supported: LOCK_VERSION,
            });
        }
        if !self.manifest_hash.starts_with("sha256:") {
            return Err(ConfigError::Validation(
                "manifest hash must use sha256".to_owned(),
            ));
        }
        if !self.workspace_hash.starts_with("sha256:") {
            return Err(ConfigError::Validation(
                "workspace hash must use sha256".to_owned(),
            ));
        }
        if self.platform.trim().is_empty() {
            return Err(ConfigError::Validation(
                "resolution platform cannot be empty".to_owned(),
            ));
        }
        for package in &self.packages {
            if !package.pom_checksum.starts_with("sha256:") {
                return Err(ConfigError::Validation(format!(
                    "package {}:{} has an invalid POM checksum",
                    package.group, package.artifact
                )));
            }
            if package
                .artifact_checksum
                .as_ref()
                .is_some_and(|checksum| !checksum.starts_with("sha256:"))
            {
                return Err(ConfigError::Validation(format!(
                    "package {}:{} has an invalid artifact checksum",
                    package.group, package.artifact
                )));
            }
        }
        Ok(())
    }

    /// Serialize a validated lockfile deterministically.
    ///
    /// # Errors
    ///
    /// Returns an error when validation or TOML serialization fails.
    pub fn to_toml(&self) -> Result<String, ConfigError> {
        self.validate()?;
        Ok(format!(
            "# Auto-generated by jman. DO NOT EDIT.\n{}",
            toml::to_string_pretty(self)?
        ))
    }
}

fn validate_java_name(label: &str, value: &str) -> Result<(), ConfigError> {
    if value.split('.').all(is_java_identifier) {
        Ok(())
    } else {
        Err(ConfigError::Validation(format!(
            "{label} `{value}` is not a valid dotted Java identifier"
        )))
    }
}

fn is_java_identifier(part: &str) -> bool {
    let mut chars = part.chars();
    chars
        .next()
        .is_some_and(|first| first == '_' || first == '$' || first.is_alphabetic())
        && chars
            .all(|character| character == '_' || character == '$' || character.is_alphanumeric())
}

fn validate_ga(coordinate: &str) -> Result<(), ConfigError> {
    let mut parts = coordinate.split(':');
    let valid = parts.next().is_some_and(|part| !part.is_empty())
        && parts.next().is_some_and(|part| !part.is_empty())
        && parts.next().is_none();
    if valid {
        Ok(())
    } else {
        Err(ConfigError::Validation(format!(
            "dependency `{coordinate}` must use group:artifact notation"
        )))
    }
}

fn validate_jdk_vendor(vendor: &str) -> Result<(), ConfigError> {
    if !vendor.is_empty()
        && vendor
            .bytes()
            .all(|value| value.is_ascii_lowercase() || value.is_ascii_digit() || value == b'-')
        && !vendor.starts_with('-')
        && !vendor.ends_with('-')
    {
        Ok(())
    } else {
        Err(ConfigError::Validation(format!(
            "JDK vendor `{vendor}` must be a lowercase identifier"
        )))
    }
}

/// Return the Java feature version from a major or exact JDK version.
#[must_use]
pub fn java_feature_version(version: &str) -> Option<u16> {
    let digit_count = version.bytes().take_while(u8::is_ascii_digit).count();
    if digit_count == 0 {
        return None;
    }
    let major = version[..digit_count]
        .parse::<u16>()
        .ok()
        .filter(|major| *major > 0)?;
    let suffix = &version[digit_count..];
    if let Some(update) = suffix.strip_prefix('u') {
        return (major == 8 && update.as_bytes().first().is_some_and(u8::is_ascii_digit))
            .then_some(major);
    }
    if !suffix.is_empty() && !matches!(suffix.as_bytes()[0], b'.' | b'+' | b'-') {
        return None;
    }
    Some(major)
}

/// Find the effective project-local Java selection.
///
/// JMAN configuration has format-level precedence, followed by `.sdkmanrc`
/// and then `.java-version`. Within one format, the nearest ancestor wins.
///
/// # Errors
///
/// Returns an error when a discovered selection file is unreadable or its
/// Java value is invalid.
pub fn find_java_selection(directory: &Path) -> Result<Option<JavaSelection>, ConfigError> {
    for ancestor in directory.ancestors() {
        let path = ancestor.join("jman.toml");
        if !path.is_file() {
            continue;
        }
        let document = ManifestFile::read(&path)?;
        if let Some(toolchain) = document.toolchain() {
            return Ok(Some(JavaSelection {
                path,
                version: toolchain.jdk.clone(),
                vendor: Some(toolchain.vendor.clone()),
                source: if document.is_project() {
                    JavaSelectionSource::JmanProject
                } else {
                    JavaSelectionSource::JmanDirectory
                },
            }));
        }
    }
    for ancestor in directory.ancestors() {
        let path = ancestor.join(".sdkmanrc");
        if !path.is_file() {
            continue;
        }
        let contents = read_java_selection_file(&path)?;
        if let Some(value) = sdkman_java_value(&contents) {
            let (version, vendor) = parse_sdkman_java(value, &path)?;
            return Ok(Some(JavaSelection {
                path,
                version,
                vendor: Some(vendor),
                source: JavaSelectionSource::Sdkmanrc,
            }));
        }
    }
    for ancestor in directory.ancestors() {
        let path = ancestor.join(".java-version");
        if !path.is_file() {
            continue;
        }
        let contents = read_java_selection_file(&path)?;
        let value = contents
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty() && !line.starts_with('#'))
            .ok_or_else(|| java_selection_error(&path, "the file is empty"))?;
        let (version, vendor) = parse_jenv_java(value, &path)?;
        return Ok(Some(JavaSelection {
            path,
            version,
            vendor,
            source: JavaSelectionSource::JavaVersion,
        }));
    }
    Ok(None)
}

fn read_java_selection_file(path: &Path) -> Result<String, ConfigError> {
    fs::read_to_string(path).map_err(|source| ConfigError::Read {
        path: path.to_owned(),
        source,
    })
}

fn sdkman_java_value(contents: &str) -> Option<&str> {
    contents.lines().rev().find_map(|line| {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            return None;
        }
        let (key, value) = line.split_once('=')?;
        (key.trim() == "java").then(|| value.trim())
    })
}

fn parse_sdkman_java(value: &str, path: &Path) -> Result<(String, String), ConfigError> {
    let value = selection_token(value, path)?;
    let (version, vendor) = value.rsplit_once('-').ok_or_else(|| {
        java_selection_error(
            path,
            "SDKMAN Java candidates must include a distribution suffix, such as `21.0.8-tem`",
        )
    })?;
    let version = normalize_compatibility_version(version);
    validate_compatibility_version(&version, path)?;
    let vendor = normalize_compatibility_vendor(vendor)
        .unwrap_or_else(|| vendor.trim().to_ascii_lowercase().replace('_', "-"));
    validate_compatibility_vendor(&vendor, path)?;
    Ok((version, vendor))
}

fn parse_jenv_java(value: &str, path: &Path) -> Result<(String, Option<String>), ConfigError> {
    let value = selection_token(value, path)?;
    let mut version = value;
    let mut vendor = None;
    if let Some((prefix, remainder)) = value.split_once('-') {
        if let Some(normalized) = normalize_compatibility_vendor(prefix) {
            version = remainder;
            vendor = Some(normalized);
        }
    }
    if vendor.is_none() {
        if let Some((remainder, suffix)) = value.rsplit_once('-') {
            if let Some(normalized) = normalize_compatibility_vendor(suffix) {
                version = remainder;
                vendor = Some(normalized);
            }
        }
    }
    let version = normalize_compatibility_version(version);
    validate_compatibility_version(&version, path)?;
    Ok((version, vendor))
}

fn selection_token<'a>(value: &'a str, path: &Path) -> Result<&'a str, ConfigError> {
    let value = value.trim();
    if value.is_empty() {
        return Err(java_selection_error(path, "the Java version is empty"));
    }
    if value.split_whitespace().count() != 1 {
        return Err(java_selection_error(
            path,
            "the Java version must be one token",
        ));
    }
    Ok(value)
}

fn normalize_compatibility_version(version: &str) -> String {
    let Some(legacy) = version.strip_prefix("1.") else {
        return version.to_owned();
    };
    let major_length = legacy.bytes().take_while(u8::is_ascii_digit).count();
    if major_length == 0 {
        return version.to_owned();
    }
    let major = &legacy[..major_length];
    let suffix = &legacy[major_length..];
    if suffix.is_empty() {
        return major.to_owned();
    }
    let update = suffix
        .strip_prefix(".0_")
        .or_else(|| suffix.strip_prefix(".0."));
    update.map_or_else(|| major.to_owned(), |update| format!("{major}u{update}"))
}

fn validate_compatibility_version(version: &str, path: &Path) -> Result<(), ConfigError> {
    java_feature_version(version).ok_or_else(|| {
        java_selection_error(path, format!("unsupported Java version `{version}`"))
    })?;
    Ok(())
}

fn normalize_compatibility_vendor(vendor: &str) -> Option<String> {
    let vendor = vendor.trim().to_ascii_lowercase().replace('_', "-");
    let normalized = match vendor.as_str() {
        "adopt" | "adoptopenjdk" | "adpt" | "tem" | "temurin" | "temurin64" => "temurin",
        "amazon" | "amzn" | "cor" | "corretto" | "corretto64" => "corretto",
        "graalce" | "graalvm-community" => "graalvm-community",
        "graal" | "graalvm" | "graalvm64" => "graalvm",
        "lib" | "liberica" | "librca" => "liberica",
        "ms" | "microsoft" => "microsoft",
        "open" | "openjdk" | "openjdk64" | "oracle-open-jdk" => "openjdk",
        "oracle" | "oracle64" => "oracle",
        "sap" | "sap-machine" | "sapmachine" | "sapmchn" => "sapmachine",
        "sem" | "semeru" => "semeru",
        "kona" => "kona",
        "nik" => "nik",
        "zulu" | "zulu64" => "zulu",
        _ => return None,
    };
    Some(normalized.to_owned())
}

fn validate_compatibility_vendor(vendor: &str, path: &Path) -> Result<(), ConfigError> {
    if !vendor.is_empty()
        && vendor
            .bytes()
            .all(|value| value.is_ascii_lowercase() || value.is_ascii_digit() || value == b'-')
        && !vendor.starts_with('-')
        && !vendor.ends_with('-')
    {
        Ok(())
    } else {
        Err(java_selection_error(
            path,
            format!("unsupported Java distribution `{vendor}`"),
        ))
    }
}

fn java_selection_error(path: &Path, message: impl Into<String>) -> ConfigError {
    ConfigError::JavaSelection {
        path: path.to_owned(),
        message: message.into(),
    }
}

fn validate_publishing(publishing: &Publishing) -> Result<(), ConfigError> {
    for (label, value) in [
        ("publishing name", publishing.name.as_deref()),
        ("publishing description", publishing.description.as_deref()),
        ("publishing URL", publishing.url.as_deref()),
    ] {
        if value.is_some_and(|value| value.trim().is_empty()) {
            return Err(ConfigError::Validation(format!("{label} cannot be empty")));
        }
    }
    for license in &publishing.licenses {
        if license.name.trim().is_empty() || license.url.trim().is_empty() {
            return Err(ConfigError::Validation(
                "publishing licenses require non-empty name and URL".to_owned(),
            ));
        }
    }
    for developer in &publishing.developers {
        if developer.name.trim().is_empty() {
            return Err(ConfigError::Validation(
                "publishing developers require a non-empty name".to_owned(),
            ));
        }
    }
    if let Some(scm) = &publishing.scm {
        if scm.connection.trim().is_empty()
            || scm.developer_connection.trim().is_empty()
            || scm.url.trim().is_empty()
        {
            return Err(ConfigError::Validation(
                "publishing SCM requires connection, developer-connection, and URL".to_owned(),
            ));
        }
    }
    Ok(())
}

fn validate_audit(audit: &Audit) -> Result<(), ConfigError> {
    let mut identifiers = std::collections::BTreeSet::new();
    for suppression in &audit.suppressions {
        if suppression.id.trim().is_empty() {
            return Err(ConfigError::Validation(
                "audit suppression ID cannot be empty".to_owned(),
            ));
        }
        if !identifiers.insert(suppression.id.as_str()) {
            return Err(ConfigError::Validation(format!(
                "duplicate audit suppression `{}`",
                suppression.id
            )));
        }
        if suppression.reason.trim().is_empty() {
            return Err(ConfigError::Validation(format!(
                "audit suppression `{}` requires a reason",
                suppression.id
            )));
        }
        if !is_iso_date(&suppression.expires) {
            return Err(ConfigError::Validation(format!(
                "audit suppression `{}` expiry must use a valid YYYY-MM-DD date",
                suppression.id
            )));
        }
    }
    Ok(())
}

fn validate_coverage(coverage: &Coverage) -> Result<(), ConfigError> {
    if coverage.engine != "jacoco" {
        return Err(ConfigError::Validation(format!(
            "unsupported coverage engine `{}`; expected `jacoco`",
            coverage.engine
        )));
    }
    for (label, threshold) in [
        ("line", coverage.minimum_line),
        ("branch", coverage.minimum_branch),
    ] {
        if threshold.is_some_and(|threshold| threshold > 100) {
            return Err(ConfigError::Validation(format!(
                "coverage minimum-{label} must be between 0 and 100"
            )));
        }
    }
    if coverage
        .include
        .iter()
        .chain(&coverage.exclude)
        .any(|pattern| pattern.trim().is_empty())
    {
        return Err(ConfigError::Validation(
            "coverage include and exclude patterns cannot be empty".to_owned(),
        ));
    }
    let unique_formats = coverage
        .formats
        .iter()
        .collect::<std::collections::BTreeSet<_>>();
    if unique_formats.len() != coverage.formats.len() {
        return Err(ConfigError::Validation(
            "coverage formats cannot contain duplicates".to_owned(),
        ));
    }
    Ok(())
}

fn is_iso_date(value: &str) -> bool {
    let mut parts = value.split('-');
    let Some(year) = parts.next().and_then(|part| part.parse::<u32>().ok()) else {
        return false;
    };
    let Some(month) = parts.next().and_then(|part| part.parse::<u32>().ok()) else {
        return false;
    };
    let Some(day) = parts.next().and_then(|part| part.parse::<u32>().ok()) else {
        return false;
    };
    if parts.next().is_some() || value.len() != 10 || year == 0 || !(1..=12).contains(&month) {
        return false;
    }
    let leap = year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400));
    let days = match month {
        2 if leap => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    };
    (1..=days).contains(&day)
}

fn default_packaging() -> String {
    "jar".to_owned()
}

fn default_encoding() -> String {
    "UTF-8".to_owned()
}

fn default_dependency_type() -> String {
    "jar".to_owned()
}

fn default_compile_scope() -> String {
    "compile".to_owned()
}

fn default_coverage_engine() -> String {
    "jacoco".to_owned()
}

fn is_jacoco(value: &str) -> bool {
    value == "jacoco"
}

fn default_jdk_vendor() -> String {
    "temurin".to_owned()
}

fn is_jar(value: &str) -> bool {
    value == "jar"
}

#[allow(clippy::trivially_copy_pass_by_ref)]
fn is_false(value: &bool) -> bool {
    !value
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_is_deterministic_and_round_trips() {
        let manifest = Manifest {
            manifest_version: MANIFEST_VERSION,
            project: Project {
                group: "com.example".to_owned(),
                name: "demo".to_owned(),
                version: "0.1".to_owned(),
                java_release: 21,
                packaging: "jar".to_owned(),
                modules: vec!["core".to_owned(), "service".to_owned()],
                main_class: Some("com.example.Application".to_owned()),
            },
            toolchain: Some(Toolchain {
                jdk: "21".to_owned(),
                vendor: "temurin".to_owned(),
            }),
            maven: None,
            build: Some(Build {
                encoding: "UTF-8".to_owned(),
                compiler_args: vec!["-parameters".to_owned()],
            }),
            test: None,
            publishing: Some(Publishing {
                name: Some("Demo".to_owned()),
                description: Some("Example project".to_owned()),
                url: Some("https://example.test/demo".to_owned()),
                licenses: vec![License {
                    name: "Apache-2.0".to_owned(),
                    url: "https://www.apache.org/licenses/LICENSE-2.0.txt".to_owned(),
                    distribution: Some("repo".to_owned()),
                }],
                developers: vec![Developer {
                    id: Some("developer".to_owned()),
                    name: "Developer".to_owned(),
                    email: None,
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
            audit: None,
            repositories: vec![Repository {
                id: "central".to_owned(),
                url: "https://repo.maven.apache.org/maven2".to_owned(),
            }],
            dependencies: Dependencies {
                compile: BTreeMap::from([("org.example:example".to_owned(), "1.0.0".to_owned())]),
                ..Dependencies::default()
            },
            annotation_processors: BTreeMap::new(),
            path_dependencies: BTreeMap::from([("com.example:core".to_owned(), "core".to_owned())]),
        };

        let text = manifest.to_toml().expect("valid manifest");
        let parsed: Manifest = toml::from_str(&text).expect("round trip");
        assert_eq!(manifest, parsed);
        assert!(text.contains("[dependencies.compile]"));
        assert!(text.contains("modules = ["));
        assert!(text.contains("[path-dependencies]"));
    }

    #[test]
    fn toolchain_only_manifest_is_valid_but_not_a_build_manifest() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let path = directory.path().join("jman.toml");
        fs::write(
            &path,
            r#"manifest-version = 1

[toolchain]
jdk = "25"
vendor = "zulu"
"#,
        )
        .expect("toolchain manifest");

        let document = ManifestFile::read(&path).expect("valid jman.toml");
        assert!(!document.is_project());
        assert_eq!(document.toolchain().expect("toolchain").jdk, "25");
        assert!(matches!(
            Manifest::read(&path),
            Err(ConfigError::ToolchainOnly { .. })
        ));
    }

    #[test]
    fn toolchain_only_manifest_accepts_exact_versions_without_a_patch_component() {
        let manifest = ToolchainManifest {
            manifest_version: MANIFEST_VERSION,
            toolchain: Toolchain {
                jdk: "27+35".to_owned(),
                vendor: "zulu".to_owned(),
            },
        };

        assert!(manifest.validate().is_ok());
        assert!(manifest
            .to_toml()
            .expect("valid manifest")
            .contains("jdk = \"27+35\""));
    }

    #[test]
    fn toolchain_only_manifest_rejects_project_configuration_without_project() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let path = directory.path().join("jman.toml");
        fs::write(
            &path,
            r#"manifest-version = 1

[toolchain]
jdk = "25"
vendor = "zulu"

[dependencies.compile]
"org.example:library" = "1"
"#,
        )
        .expect("invalid manifest");

        assert!(matches!(
            ManifestFile::read(&path),
            Err(ConfigError::Parse { .. })
        ));
    }

    #[test]
    fn java_selection_files_follow_format_precedence_and_ancestor_discovery() {
        let root = tempfile::tempdir().expect("temporary directory");
        let nested = root.path().join("services/orders");
        fs::create_dir_all(&nested).expect("nested directory");
        fs::write(nested.join(".java-version"), "temurin64-17.0.12\n").expect("jenv selection");
        fs::write(root.path().join(".sdkmanrc"), "java=21.0.8-zulu\n").expect("SDKMAN selection");
        fs::write(
            root.path().join("jman.toml"),
            r#"manifest-version = 1

[toolchain]
jdk = "25"
vendor = "corretto"
"#,
        )
        .expect("JMAN selection");

        let selection = find_java_selection(&nested)
            .expect("selection")
            .expect("JMAN selection");
        assert_eq!(selection.version, "25");
        assert_eq!(selection.vendor.as_deref(), Some("corretto"));
        assert_eq!(selection.source, JavaSelectionSource::JmanDirectory);

        fs::remove_file(root.path().join("jman.toml")).expect("remove JMAN selection");
        let selection = find_java_selection(&nested)
            .expect("selection")
            .expect("SDKMAN selection");
        assert_eq!(selection.version, "21.0.8");
        assert_eq!(selection.vendor.as_deref(), Some("zulu"));
        assert_eq!(selection.source, JavaSelectionSource::Sdkmanrc);

        fs::remove_file(root.path().join(".sdkmanrc")).expect("remove SDKMAN selection");
        let selection = find_java_selection(&nested)
            .expect("selection")
            .expect("jenv selection");
        assert_eq!(selection.version, "17.0.12");
        assert_eq!(selection.vendor.as_deref(), Some("temurin"));
        assert_eq!(selection.source, JavaSelectionSource::JavaVersion);
        assert_eq!(selection.path, nested.join(".java-version"));
    }

    #[test]
    fn java_selection_files_normalize_legacy_and_vendor_aliases() {
        let directory = tempfile::tempdir().expect("temporary directory");
        fs::write(directory.path().join(".java-version"), "1.8.0_402\n")
            .expect("legacy jenv selection");
        let selection = find_java_selection(directory.path())
            .expect("selection")
            .expect("jenv selection");
        assert_eq!(selection.version, "8u402");
        assert_eq!(selection.vendor, None);

        fs::remove_file(directory.path().join(".java-version")).expect("remove jenv selection");
        fs::write(
            directory.path().join(".sdkmanrc"),
            "# project candidates\njava=17.0.12-librca\n",
        )
        .expect("SDKMAN selection");
        let selection = find_java_selection(directory.path())
            .expect("selection")
            .expect("SDKMAN selection");
        assert_eq!(selection.version, "17.0.12");
        assert_eq!(selection.vendor.as_deref(), Some("liberica"));
    }

    #[test]
    fn malformed_java_selection_reports_its_source_file() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let path = directory.path().join(".sdkmanrc");
        fs::write(&path, "java=not-a-candidate\n").expect("invalid selection");

        let error = find_java_selection(directory.path()).expect_err("invalid selection");

        assert!(error.to_string().contains(&path.display().to_string()));
        assert!(error.to_string().contains("unsupported Java version"));
    }

    #[test]
    fn rejects_invalid_dependency_coordinates() {
        let mut manifest = minimal_manifest();
        manifest
            .dependencies
            .compile
            .insert("missing-artifact".to_owned(), "1".to_owned());

        assert!(matches!(
            manifest.validate(),
            Err(ConfigError::Validation(message)) if message.contains("group:artifact")
        ));
    }

    #[test]
    fn accepts_provider_neutral_vendor_ids_and_rejects_unsafe_values() {
        let mut manifest = minimal_manifest();
        manifest.toolchain = Some(Toolchain {
            jdk: "21".to_owned(),
            vendor: "future-provider-jdk".to_owned(),
        });
        assert!(manifest.validate().is_ok());

        manifest.toolchain.as_mut().expect("toolchain").vendor = "https://example.test".to_owned();
        assert!(matches!(
            manifest.validate(),
            Err(ConfigError::Validation(message)) if message.contains("lowercase identifier")
        ));
    }

    #[test]
    fn rejects_incomplete_configured_publishing_entries() {
        let mut manifest = minimal_manifest();
        manifest.publishing = Some(Publishing {
            licenses: vec![License {
                name: String::new(),
                url: "https://example.test/license".to_owned(),
                distribution: None,
            }],
            ..Publishing::default()
        });
        assert!(matches!(
            manifest.validate(),
            Err(ConfigError::Validation(message)) if message.contains("licenses")
        ));

        manifest.publishing = Some(Publishing {
            developers: vec![Developer {
                id: None,
                name: "  ".to_owned(),
                email: None,
                organization: None,
                organization_url: None,
            }],
            ..Publishing::default()
        });
        assert!(matches!(
            manifest.validate(),
            Err(ConfigError::Validation(message)) if message.contains("developers")
        ));
    }

    #[test]
    fn audit_suppressions_require_a_reason_and_valid_expiry_date() {
        let mut manifest = minimal_manifest();
        manifest.audit = Some(Audit {
            suppressions: vec![AuditSuppression {
                id: "GHSA-example".to_owned(),
                reason: "Compensating input validation is deployed".to_owned(),
                expires: "2027-03-31".to_owned(),
            }],
        });
        assert!(manifest.validate().is_ok());
        let text = manifest.to_toml().expect("audit configuration");
        assert!(text.contains("[[audit.suppressions]]"));
        assert_eq!(
            toml::from_str::<Manifest>(&text)
                .expect("round-trip manifest")
                .audit,
            manifest.audit
        );

        manifest
            .audit
            .as_mut()
            .expect("audit configuration")
            .suppressions[0]
            .reason
            .clear();
        assert!(matches!(
            manifest.validate(),
            Err(ConfigError::Validation(message)) if message.contains("reason")
        ));
        let suppression = &mut manifest
            .audit
            .as_mut()
            .expect("audit configuration")
            .suppressions[0];
        suppression.reason = "accepted risk".to_owned();
        suppression.expires = "2027-02-29".to_owned();
        assert!(matches!(
            manifest.validate(),
            Err(ConfigError::Validation(message)) if message.contains("expiry")
        ));
    }

    #[test]
    fn programmatic_maven_dependency_metadata_defaults_to_jar() {
        let metadata = MavenDependencyMetadata::default();
        assert_eq!(metadata.dependency_type, "jar");
        assert_eq!(metadata, toml::from_str("").expect("default metadata"));
    }

    fn minimal_manifest() -> Manifest {
        Manifest {
            manifest_version: MANIFEST_VERSION,
            project: Project {
                group: "com.example".to_owned(),
                name: "demo".to_owned(),
                version: "0.1".to_owned(),
                java_release: 21,
                packaging: "jar".to_owned(),
                modules: Vec::new(),
                main_class: None,
            },
            toolchain: None,
            maven: None,
            build: None,
            test: None,
            publishing: None,
            audit: None,
            repositories: Vec::new(),
            dependencies: Dependencies::default(),
            annotation_processors: BTreeMap::new(),
            path_dependencies: BTreeMap::new(),
        }
    }

    #[test]
    fn coverage_configuration_round_trips_and_validates_thresholds() {
        let mut manifest = minimal_manifest();
        manifest.test = Some(Test {
            coverage: Some(Coverage {
                enabled: true,
                engine: "jacoco".to_owned(),
                formats: vec![CoverageFormat::Summary, CoverageFormat::Html],
                minimum_line: Some(80),
                minimum_branch: Some(70),
                include: vec!["com.example.*".to_owned()],
                exclude: vec!["com.example.generated.*".to_owned()],
            }),
        });

        let serialized = manifest.to_toml().expect("serialize coverage");
        assert!(serialized.contains("[test.coverage]"));
        assert!(serialized.contains("formats = ["));
        assert!(serialized.contains("\"summary\""));
        assert!(serialized.contains("\"html\""));
        assert_eq!(
            toml::from_str::<Manifest>(&serialized).expect("parse coverage"),
            manifest
        );

        manifest
            .test
            .as_mut()
            .and_then(|test| test.coverage.as_mut())
            .expect("coverage")
            .minimum_line = Some(101);
        assert!(manifest
            .validate()
            .expect_err("reject threshold")
            .to_string()
            .contains("between 0 and 100"));
    }
}
