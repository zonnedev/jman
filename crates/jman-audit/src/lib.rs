//! Provider-neutral vulnerability auditing for resolved Maven packages.

mod cache;
mod model;
mod osv;
mod provider;

pub use cache::{AuditOptions, Auditor};
pub use model::{Advisory, AuditPackage, AuditResult, AuditSource, Finding, Severity};
pub use osv::{OsvProvider, OSV_API_URL};
pub use provider::{AuditError, VulnerabilityProvider};
