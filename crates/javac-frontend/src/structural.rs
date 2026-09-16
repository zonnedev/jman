use crate::{SemanticDiagnostic, SemanticSymbol};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceSource {
    pub file_name: String,
    pub source: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StructuralFile {
    pub file_name: String,
    pub package_name: String,
    pub imports: Vec<String>,
    pub symbols: Vec<SemanticSymbol>,
    pub diagnostics: Vec<SemanticDiagnostic>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceParseResult {
    pub files: Vec<StructuralFile>,
}
