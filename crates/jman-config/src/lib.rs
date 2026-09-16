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

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
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
            if toolchain.vendor != "temurin" {
                return Err(ConfigError::Validation(format!(
                    "unsupported JDK vendor `{}`; this jman version supports `temurin`",
                    toolchain.vendor
                )));
            }
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
            repositories: Vec::new(),
            dependencies: Dependencies::default(),
            annotation_processors: BTreeMap::new(),
            path_dependencies: BTreeMap::new(),
        }
    }
}
