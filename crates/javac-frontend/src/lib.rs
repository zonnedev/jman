//! Safe Rust facade for the versioned Java frontend native ABI.

mod editor;
mod format;
#[cfg(feature = "native-ffi")]
mod native;
mod project_state;
mod semantic;
mod structural;
mod symbol_index;

pub use format::FormatResult;
#[cfg(feature = "native-ffi")]
pub use native::{
    Frontend, FrontendError, ProjectSession, decode_editor_query_result, decode_format_result,
    decode_semantic_result,
};
pub use project_state::{ProjectChange, ProjectState};
pub use semantic::{DependencyGraph, SemanticDiagnostic, SemanticResult, SemanticSymbol};
pub use structural::{StructuralFile, WorkspaceParseResult, WorkspaceSource};
pub use symbol_index::{RenameEdit, RenameError, SymbolIndex, SymbolLocation, is_java_identifier};

/// ABI version implemented by this crate.
pub const ABI_VERSION: u32 = 5;
pub use editor::{
    EditorCompletion, EditorDefinition, EditorHover, EditorQueryResult, EditorSignature,
};
