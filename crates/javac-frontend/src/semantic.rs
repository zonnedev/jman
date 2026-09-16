use std::collections::{HashMap, HashSet, VecDeque};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SemanticResult {
    pub package_name: String,
    pub symbols: Vec<SemanticSymbol>,
    pub diagnostics: Vec<SemanticDiagnostic>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SemanticSymbol {
    pub role: String,
    pub kind: String,
    pub name: String,
    pub qualified_name: String,
    #[serde(default)]
    pub symbol_id: String,
    pub start: u64,
    pub end: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SemanticDiagnostic {
    pub kind: String,
    pub code: String,
    pub start: u64,
    pub end: u64,
    pub line: u64,
    pub column: u64,
    pub message: String,
}

#[derive(Debug, Default)]
pub struct DependencyGraph {
    documents: HashMap<String, DocumentFacts>,
    referenced_by: HashMap<String, HashSet<String>>,
}

#[derive(Debug, Default)]
struct DocumentFacts {
    declarations: HashSet<String>,
    references: HashSet<String>,
}

impl DependencyGraph {
    pub fn update_document(
        &mut self,
        document: impl Into<String>,
        result: &SemanticResult,
    ) -> HashSet<String> {
        let document = document.into();
        let facts = DocumentFacts::from_result(result);
        let changed_declarations = self
            .documents
            .get(&document)
            .map(|previous| {
                facts
                    .declarations
                    .symmetric_difference(&previous.declarations)
                    .cloned()
                    .collect()
            })
            .unwrap_or_else(|| facts.declarations.clone());
        let affected = self.affected_documents(&document, changed_declarations);
        self.remove_edges(&document);
        for reference in &facts.references {
            self.referenced_by
                .entry(reference.clone())
                .or_default()
                .insert(document.clone());
        }
        self.documents.insert(document, facts);
        affected
    }

    pub fn remove_document(&mut self, document: &str) -> HashSet<String> {
        let declarations = self
            .documents
            .get(document)
            .map(|facts| facts.declarations.clone())
            .unwrap_or_default();
        let affected = self.affected_documents(document, declarations);
        self.remove_edges(document);
        self.documents.remove(document);
        affected
    }

    fn affected_documents(
        &self,
        changed_document: &str,
        changed_declarations: HashSet<String>,
    ) -> HashSet<String> {
        let mut affected = HashSet::from([changed_document.to_owned()]);
        let mut declarations: VecDeque<String> = changed_declarations.into_iter().collect();
        let mut visited_declarations = HashSet::new();
        while let Some(declaration) = declarations.pop_front() {
            if !visited_declarations.insert(declaration.clone()) {
                continue;
            }
            let Some(dependents) = self.referenced_by.get(&declaration) else {
                continue;
            };
            for dependent in dependents {
                if affected.insert(dependent.clone())
                    && let Some(facts) = self.documents.get(dependent)
                {
                    declarations.extend(facts.declarations.iter().cloned());
                }
            }
        }
        affected
    }

    fn remove_edges(&mut self, document: &str) {
        let Some(previous) = self.documents.get(document) else {
            return;
        };
        for reference in &previous.references {
            if let Some(documents) = self.referenced_by.get_mut(reference) {
                documents.remove(document);
                if documents.is_empty() {
                    self.referenced_by.remove(reference);
                }
            }
        }
    }
}

impl DocumentFacts {
    fn from_result(result: &SemanticResult) -> Self {
        let mut facts = Self::default();
        for symbol in &result.symbols {
            let identity = if symbol.symbol_id.is_empty() {
                &symbol.qualified_name
            } else {
                &symbol.symbol_id
            };
            if identity.is_empty() {
                continue;
            }
            match symbol.role.as_str() {
                "declaration" => {
                    facts.declarations.insert(identity.clone());
                }
                "reference" => {
                    facts.references.insert(identity.clone());
                }
                _ => {}
            }
        }
        facts
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn result(declarations: &[&str], references: &[&str]) -> SemanticResult {
        let symbols = declarations
            .iter()
            .map(|name| symbol("declaration", name))
            .chain(references.iter().map(|name| symbol("reference", name)))
            .collect();
        SemanticResult {
            package_name: String::new(),
            symbols,
            diagnostics: Vec::new(),
        }
    }

    fn symbol(role: &str, qualified_name: &str) -> SemanticSymbol {
        SemanticSymbol {
            role: role.to_owned(),
            kind: "class".to_owned(),
            name: qualified_name.rsplit('.').next().unwrap().to_owned(),
            qualified_name: qualified_name.to_owned(),
            symbol_id: qualified_name.to_owned(),
            start: 0,
            end: 1,
        }
    }

    #[test]
    fn body_only_changes_do_not_invalidate_dependents() {
        let mut graph = DependencyGraph::default();
        graph.update_document("Model.java", &result(&["demo.Model"], &[]));
        graph.update_document("Service.java", &result(&["demo.Service"], &["demo.Model"]));
        graph.update_document(
            "Controller.java",
            &result(&["demo.Controller"], &["demo.Service"]),
        );
        graph.update_document("Unrelated.java", &result(&["demo.Unrelated"], &[]));

        let affected = graph.update_document(
            "Model.java",
            &result(&["demo.Model"], &["java.lang.String"]),
        );

        assert_eq!(affected, HashSet::from(["Model.java".to_owned()]));
        assert!(!affected.contains("Unrelated.java"));
    }

    #[test]
    fn declaration_changes_invalidate_transitive_dependents() {
        let mut graph = DependencyGraph::default();
        graph.update_document("Model.java", &result(&["demo.Model"], &[]));
        graph.update_document("Service.java", &result(&["demo.Service"], &["demo.Model"]));
        graph.update_document(
            "Controller.java",
            &result(&["demo.Controller"], &["demo.Service"]),
        );

        let affected = graph.update_document("Model.java", &result(&["demo.RenamedModel"], &[]));

        assert_eq!(
            affected,
            HashSet::from([
                "Model.java".to_owned(),
                "Service.java".to_owned(),
                "Controller.java".to_owned()
            ])
        );
    }

    #[test]
    fn removing_a_document_invalidates_its_dependents() {
        let mut graph = DependencyGraph::default();
        graph.update_document("Api.java", &result(&["demo.Api"], &[]));
        graph.update_document("Client.java", &result(&[], &["demo.Api"]));

        assert_eq!(
            graph.remove_document("Api.java"),
            HashSet::from(["Api.java".to_owned(), "Client.java".to_owned()])
        );
    }
}
