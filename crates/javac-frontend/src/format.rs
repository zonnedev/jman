use serde::{Deserialize, Serialize};

use crate::SemanticDiagnostic;

/// Canonical Java source produced by the native javac formatter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FormatResult {
    pub source: String,
    pub diagnostics: Vec<SemanticDiagnostic>,
}
