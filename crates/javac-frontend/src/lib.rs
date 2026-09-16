//! Safe Rust facade for the versioned Java frontend native ABI.

mod editor;
#[cfg(feature = "native-ffi")]
mod native;
mod project_state;
mod semantic;
mod structural;
mod symbol_index;

#[cfg(feature = "native-ffi")]
pub use native::{Frontend, FrontendError, ProjectSession};
pub use project_state::{ProjectChange, ProjectState};
pub use semantic::{DependencyGraph, SemanticDiagnostic, SemanticResult, SemanticSymbol};
pub use structural::{StructuralFile, WorkspaceParseResult, WorkspaceSource};
pub use symbol_index::{RenameEdit, RenameError, SymbolIndex, SymbolLocation, is_java_identifier};

/// ABI version implemented by this crate.
pub const ABI_VERSION: u32 = 3;
pub use editor::{
    EditorCompletion, EditorDefinition, EditorHover, EditorQueryResult, EditorSignature,
};
