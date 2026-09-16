use std::{collections::BTreeMap, fmt};

use serde::{Deserialize, Serialize};

use crate::ResolverError;

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct Coordinate {
    pub group: String,
    pub artifact: String,
    pub version: String,
    pub extension: String,
    pub classifier: Option<String>,
}

impl Coordinate {
    pub fn pom(
        group: impl Into<String>,
        artifact: impl Into<String>,
        version: impl Into<String>,
    ) -> Self {
        Self {
            group: group.into(),
            artifact: artifact.into(),
            version: version.into(),
            extension: "pom".to_owned(),
            classifier: None,
        }
    }

    pub fn jar(
        group: impl Into<String>,
        artifact: impl Into<String>,
        version: impl Into<String>,
    ) -> Self {
        Self {
            group: group.into(),
            artifact: artifact.into(),
            version: version.into(),
            extension: "jar".to_owned(),
            classifier: None,
        }
    }

    /// Parse Maven's `group:artifact[:extension[:classifier]]:version` notation.
    ///
    /// # Errors
    ///
    /// Returns an error when the coordinate has an unsupported shape.
    pub fn parse(value: &str) -> Result<Self, ResolverError> {
        let parts: Vec<_> = value.split(':').collect();
        match parts.as_slice() {
            [group, artifact, version] if parts.iter().all(|part| !part.is_empty()) => {
                Ok(Self::jar(*group, *artifact, *version))
            }
            [group, artifact, extension, version] if parts.iter().all(|part| !part.is_empty()) => {
                Ok(Self {
                    group: (*group).to_owned(),
                    artifact: (*artifact).to_owned(),
                    version: (*version).to_owned(),
                    extension: (*extension).to_owned(),
                    classifier: None,
                })
            }
            [group, artifact, extension, classifier, version]
                if parts.iter().all(|part| !part.is_empty()) =>
            {
                Ok(Self {
                    group: (*group).to_owned(),
                    artifact: (*artifact).to_owned(),
                    version: (*version).to_owned(),
                    extension: (*extension).to_owned(),
                    classifier: Some((*classifier).to_owned()),
                })
            }
            _ => Err(ResolverError::InvalidCoordinate(value.to_owned())),
        }
    }

    #[must_use]
    pub fn ga(&self) -> String {
        format!("{}:{}", self.group, self.artifact)
    }

    #[must_use]
    pub fn management_key(&self) -> String {
        format!(
            "{}:{}:{}:{}",
            self.group,
            self.artifact,
            self.extension,
            self.classifier.as_deref().unwrap_or_default()
        )
    }

    #[must_use]
    pub fn repository_path(&self) -> String {
        let group_path = self.group.replace('.', "/");
        let classifier = self
            .classifier
            .as_ref()
            .map_or_else(String::new, |value| format!("-{value}"));
        format!(
            "{group_path}/{artifact}/{version}/{artifact}-{version}{classifier}.{extension}",
            artifact = self.artifact,
            version = self.version,
            extension = self.extension
        )
    }
}

impl fmt::Display for Coordinate {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (&*self.extension, &self.classifier) {
            ("jar", None) => {
                write!(
                    formatter,
                    "{}:{}:{}",
                    self.group, self.artifact, self.version
                )
            }
            (extension, None) => write!(
                formatter,
                "{}:{}:{extension}:{}",
                self.group, self.artifact, self.version
            ),
            (extension, Some(classifier)) => write!(
                formatter,
                "{}:{}:{extension}:{classifier}:{}",
                self.group, self.artifact, self.version
            ),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Parent {
    pub group: String,
    pub artifact: String,
    pub version: String,
    pub relative_path: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Exclusion {
    pub group: String,
    pub artifact: String,
}

impl Exclusion {
    #[must_use]
    pub fn matches(&self, dependency: &Dependency) -> bool {
        (self.group == "*" || self.group == dependency.group)
            && (self.artifact == "*" || self.artifact == dependency.artifact)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Dependency {
    pub group: String,
    pub artifact: String,
    pub version: Option<String>,
    pub scope: String,
    pub scope_explicit: bool,
    pub dependency_type: String,
    pub type_explicit: bool,
    pub classifier: Option<String>,
    pub optional: Option<bool>,
    pub exclusions: Vec<Exclusion>,
}

impl Dependency {
    #[must_use]
    pub fn ga(&self) -> String {
        format!("{}:{}", self.group, self.artifact)
    }

    #[must_use]
    pub fn management_key(&self) -> String {
        format!(
            "{}:{}:{}:{}",
            self.group,
            self.artifact,
            self.dependency_type,
            self.classifier.as_deref().unwrap_or_default()
        )
    }

    /// Convert a dependency with an effective version to an artifact coordinate.
    ///
    /// # Errors
    ///
    /// Returns an error when dependency management did not provide a version.
    pub fn coordinate(&self) -> Result<Coordinate, ResolverError> {
        let version = self
            .version
            .clone()
            .ok_or_else(|| ResolverError::MissingVersion(self.ga()))?;
        Ok(Coordinate {
            group: self.group.clone(),
            artifact: self.artifact.clone(),
            version,
            extension: type_to_extension(&self.dependency_type),
            classifier: self
                .classifier
                .clone()
                .or_else(|| (self.dependency_type == "test-jar").then(|| "tests".to_owned())),
        })
    }
}

#[must_use]
pub fn type_to_extension(dependency_type: &str) -> String {
    match dependency_type {
        "test-jar" | "bundle" | "maven-plugin" | "ejb" => "jar".to_owned(),
        other => other.to_owned(),
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Repository {
    pub id: String,
    pub url: String,
    pub releases_enabled: bool,
    pub snapshots_enabled: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AnnotationProcessor {
    pub dependency: Dependency,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RawPom {
    pub model_version: Option<String>,
    pub parent: Option<Parent>,
    pub group: Option<String>,
    pub artifact: String,
    pub version: Option<String>,
    pub packaging: String,
    pub modules: Vec<String>,
    pub properties: BTreeMap<String, String>,
    pub dependencies: Vec<Dependency>,
    pub dependency_management: Vec<Dependency>,
    pub repositories: Vec<Repository>,
    pub annotation_processors: Vec<AnnotationProcessor>,
    pub compiler_args: Vec<String>,
    pub relocation: Option<Relocation>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EffectivePom {
    pub coordinate: Coordinate,
    pub parent: Option<Coordinate>,
    pub packaging: String,
    pub modules: Vec<String>,
    pub property_templates: BTreeMap<String, String>,
    pub properties: BTreeMap<String, String>,
    pub dependencies: Vec<Dependency>,
    pub dependency_management: Vec<Dependency>,
    pub bom_imports: Vec<Coordinate>,
    pub repositories: Vec<Repository>,
    pub annotation_processors: Vec<AnnotationProcessor>,
    pub compiler_args: Vec<String>,
    pub relocation: Option<Coordinate>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Relocation {
    pub group: Option<String>,
    pub artifact: Option<String>,
    pub version: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MavenProject {
    pub source: std::path::PathBuf,
    pub effective: EffectivePom,
}
