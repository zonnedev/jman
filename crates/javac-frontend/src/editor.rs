use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EditorCompletion {
    pub label: String,
    pub kind: String,
    pub detail: String,
    pub insert_text: String,
    pub documentation: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EditorSignature {
    pub label: String,
    pub parameters: Vec<String>,
    pub return_type: String,
    pub documentation: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EditorHover {
    pub detail: String,
    pub documentation: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EditorDefinition {
    pub symbol_id: String,
    pub module: String,
    pub owner: String,
    pub name: String,
    pub descriptor: String,
    pub source_name: String,
    pub source: String,
    pub start: u64,
    pub end: u64,
    pub decompiled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EditorQueryResult {
    pub completions: Vec<EditorCompletion>,
    pub signatures: Vec<EditorSignature>,
    pub hover: Option<EditorHover>,
    pub definition: Option<EditorDefinition>,
    #[serde(default)]
    pub type_definition: Option<EditorDefinition>,
}
