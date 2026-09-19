//! Native Maven repository and dependency resolution.

mod effective;
mod graph;
mod model;
mod pom;
mod repository;
mod version;

pub use effective::{EffectiveModelBuilder, PomSource};
pub use graph::{DependencyResolver, ResolvedGraph, ResolvedPackage};
pub use model::{
    AnnotationProcessor, Coordinate, Dependency, EffectivePom, Exclusion, MavenProject, Parent,
    RawPom, Relocation, Repository,
};
pub use pom::{parse_pom, plugin_coordinates};
pub use repository::{CachedFile, RepositoryClient, VersionCatalog, MAVEN_CENTRAL_URL};
pub use version::{analyze_versions, VersionChange, VersionUpdates};

use thiserror::Error;

#[derive(Debug, Error)]
pub enum ResolverError {
    #[error("invalid Maven POM: {0}")]
    InvalidPom(String),
    #[error("invalid Maven metadata: {0}")]
    InvalidMetadata(String),
    #[error("invalid Maven coordinate: {0}")]
    InvalidCoordinate(String),
    #[error("dependency {0} has no version after dependency management")]
    MissingVersion(String),
    #[error("unresolved Maven property `{property}` in `{value}`")]
    UnresolvedProperty { property: String, value: String },
    #[error("dependency model contains a cycle: {0}")]
    ModelCycle(String),
    #[error("failed to build effective Maven model for {coordinate}: {source}")]
    ModelContext {
        coordinate: String,
        #[source]
        source: Box<ResolverError>,
    },
    #[error("resolver background task failed: {0}")]
    BackgroundTask(String),
    #[error("repository request failed for {url}: {source}")]
    Request { url: String, source: reqwest::Error },
    #[error("repository returned HTTP {status} for {url}")]
    HttpStatus {
        url: String,
        status: reqwest::StatusCode,
    },
    #[error("offline cache miss for {0}")]
    OfflineMiss(String),
    #[error("cache operation failed at {path}: {source}")]
    Cache {
        path: std::path::PathBuf,
        source: std::io::Error,
    },
}
