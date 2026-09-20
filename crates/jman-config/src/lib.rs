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
    #[error("invalid configuration: {0}")]
    Validation(String),
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
        let text = fs::read_to_string(path).map_err(|source| ConfigError::Read {
            path: path.to_owned(),
            source,
        })?;
        let manifest: Self = toml::from_str(&text).map_err(|source| ConfigError::Parse {
            path: path.to_owned(),
            source,
        })?;
        manifest.validate()?;
        Ok(manifest)
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
            let major = toolchain
                .jdk
                .split('.')
                .next()
                .and_then(|value| value.parse::<u16>().ok());
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
