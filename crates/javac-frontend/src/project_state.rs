use std::collections::HashSet;

use crate::{
    DependencyGraph, RenameEdit, RenameError, SemanticResult, SymbolIndex, SymbolLocation,
};

#[derive(Debug, Default)]
pub struct ProjectState {
    dependencies: DependencyGraph,
    symbols: SymbolIndex,
}

#[derive(Debug, PartialEq, Eq)]
pub struct ProjectChange {
    pub invalidated_documents: Vec<String>,
}

impl ProjectState {
    pub fn update_document(
        &mut self,
        document: impl Into<String>,
        result: &SemanticResult,
    ) -> ProjectChange {
        let document = document.into();
        let invalidated = self.dependencies.update_document(document.clone(), result);
        self.symbols.update_document(document, result);
        ProjectChange::new(invalidated)
    }

    pub fn remove_document(&mut self, document: &str) -> ProjectChange {
        let invalidated = self.dependencies.remove_document(document);
        self.symbols.remove_document(document);
        ProjectChange::new(invalidated)
    }

    pub fn definitions(&self, qualified_name: &str) -> &[SymbolLocation] {
        self.symbols.definitions(qualified_name)
    }

    pub fn declarations_named(&self, name: &str) -> Vec<SymbolLocation> {
        self.symbols.declarations_named(name)
    }

    pub fn related_symbol_ids(&self, symbol_id: &str) -> Vec<String> {
        self.symbols.related_symbol_ids(symbol_id)
    }

    pub fn incoming_calls(&self, symbol_id: &str) -> Vec<SymbolLocation> {
        self.symbols.incoming_calls(symbol_id)
    }

    pub fn outgoing_calls(&self, symbol_id: &str) -> Vec<SymbolLocation> {
        self.symbols.outgoing_calls(symbol_id)
    }

    pub fn document_call_edges(&self, document: &str) -> Vec<SymbolLocation> {
        self.symbols.document_call_edges(document)
    }

    pub fn direct_supertypes(&self, symbol_id: &str) -> Vec<SymbolLocation> {
        self.symbols.direct_supertypes(symbol_id)
    }

    pub fn direct_subtypes(&self, symbol_id: &str) -> Vec<SymbolLocation> {
        self.symbols.direct_subtypes(symbol_id)
    }

    pub fn references(&self, qualified_name: &str) -> &[SymbolLocation] {
        self.symbols.references(qualified_name)
    }

    pub fn symbol_at(&self, document: &str, offset: u64) -> Option<&SymbolLocation> {
        self.symbols.symbol_at(document, offset)
    }

    pub fn rename_locations(&self, qualified_name: &str) -> Vec<SymbolLocation> {
        self.symbols.rename_locations(qualified_name)
    }

    pub fn prepare_rename(
        &self,
        qualified_name: &str,
        new_name: &str,
    ) -> Result<Vec<RenameEdit>, RenameError> {
        self.symbols.prepare_rename(qualified_name, new_name)
    }

    pub fn workspace_symbols(&self, query: &str, limit: usize) -> Vec<SymbolLocation> {
        self.symbols.workspace_symbols(query, limit)
    }

    pub fn document_symbols(&self, document: &str) -> Vec<SymbolLocation> {
        self.symbols.document_symbols(document)
    }

    pub fn completion_symbols(&self, query: &str, limit: usize) -> Vec<SymbolLocation> {
        self.symbols.completion_symbols(query, limit)
    }
}

impl ProjectChange {
    fn new(documents: HashSet<String>) -> Self {
        let mut invalidated_documents: Vec<_> = documents.into_iter().collect();
        invalidated_documents.sort();
        Self {
            invalidated_documents,
        }
    }

    #[cfg(feature = "native-ffi")]
    pub fn invalidate_session(
        &self,
        session: &crate::ProjectSession,
    ) -> Result<(), crate::FrontendError> {
        for document in &self.invalidated_documents {
            session.invalidate(document)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SemanticSymbol;

    fn result(declaration: &str, references: &[&str]) -> SemanticResult {
        let name = declaration.rsplit('.').next().unwrap();
        let mut symbols = vec![SemanticSymbol {
            role: "declaration".to_owned(),
            kind: "class".to_owned(),
            name: name.to_owned(),
            qualified_name: declaration.to_owned(),
            symbol_id: declaration.to_owned(),
            start: 0,
            end: 1,
        }];
        symbols.extend(references.iter().map(|reference| SemanticSymbol {
            role: "reference".to_owned(),
            kind: "class".to_owned(),
            name: reference.rsplit('.').next().unwrap().to_owned(),
            qualified_name: (*reference).to_owned(),
            symbol_id: (*reference).to_owned(),
            start: 2,
            end: 3,
        }));
        SemanticResult {
            package_name: "demo".to_owned(),
            symbols,
            diagnostics: Vec::new(),
        }
    }

    #[test]
    fn updates_navigation_without_invalidating_dependents_for_body_changes() {
        let mut project = ProjectState::default();
        project.update_document("Model.java", &result("demo.Model", &[]));
        project.update_document("Service.java", &result("demo.Service", &["demo.Model"]));

        let change =
            project.update_document("Model.java", &result("demo.Model", &["java.lang.String"]));

        assert_eq!(change.invalidated_documents, ["Model.java"]);
        assert_eq!(project.definitions("demo.Model").len(), 1);
        assert_eq!(project.references("demo.Model").len(), 1);
        assert_eq!(project.workspace_symbols("serv", 10)[0].name, "Service");
    }
}
