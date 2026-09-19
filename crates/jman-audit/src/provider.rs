use std::{future::Future, path::PathBuf, pin::Pin};

use thiserror::Error;

use crate::{AuditPackage, Finding};

#[derive(Debug, Error)]
pub enum AuditError {
    #[error("audit provider request failed for {url}: {source}")]
    Request { url: String, source: reqwest::Error },
    #[error("audit provider returned HTTP {status} for {url}")]
    HttpStatus {
        url: String,
        status: reqwest::StatusCode,
    },
    #[error("invalid audit provider response: {0}")]
    InvalidResponse(String),
    #[error("audit cache operation failed at {path}: {source}")]
    Cache {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("invalid audit cache at {path}: {message}")]
    InvalidCache { path: PathBuf, message: String },
    #[error("offline audit cache miss for the resolved dependency graph")]
    OfflineMiss,
}

pub trait VulnerabilityProvider: Send + Sync {
    fn name(&self) -> &'static str;

    fn cache_identity(&self) -> String {
        self.name().to_owned()
    }

    fn query<'a>(
        &'a self,
        packages: &'a [AuditPackage],
    ) -> Pin<Box<dyn Future<Output = Result<Vec<Finding>, AuditError>> + Send + 'a>>;
}
