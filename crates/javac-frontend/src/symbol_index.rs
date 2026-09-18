use std::collections::HashMap;

use crate::SemanticResult;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SymbolLocation {
    pub document: String,
    pub role: String,
    pub kind: String,
    pub name: String,
    pub qualified_name: String,
    pub symbol_id: String,
    pub start: u64,
    pub end: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenameEdit {
    pub document: String,
    pub start: u64,
    pub end: u64,
    pub new_text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RenameError {
    InvalidIdentifier,
    DefinitionNotFound,
    AmbiguousDefinition,
}

#[derive(Debug, Default)]
pub struct SymbolIndex {
    documents: HashMap<String, Vec<SymbolLocation>>,
    definitions: HashMap<String, Vec<SymbolLocation>>,
    references: HashMap<String, Vec<SymbolLocation>>,
    override_families: HashMap<String, String>,
    document_override_families: HashMap<String, Vec<(String, String)>>,
}

impl SymbolIndex {
    pub fn update_document(&mut self, document: impl Into<String>, result: &SemanticResult) {
        let document = document.into();
        self.remove_document(&document);
        let relationships: Vec<_> = result
            .symbols
            .iter()
            .filter(|symbol| symbol.role == "override_family")
            .map(|symbol| (symbol.symbol_id.clone(), symbol.qualified_name.clone()))
            .collect();
        let locations: Vec<_> = result
            .symbols
            .iter()
            .filter(|symbol| !symbol.qualified_name.is_empty() && symbol.role != "override_family")
            .map(|symbol| SymbolLocation {
                document: document.clone(),
                role: symbol.role.clone(),
                kind: symbol.kind.clone(),
                name: symbol.name.clone(),
                qualified_name: symbol.qualified_name.clone(),
                symbol_id: if symbol.symbol_id.is_empty() {
                    symbol.qualified_name.clone()
                } else {
                    symbol.symbol_id.clone()
                },
                start: symbol.start,
                end: symbol.end,
            })
            .collect();
        for location in &locations {
            let index = if location.role == "declaration" {
                &mut self.definitions
            } else {
                &mut self.references
            };
            index
                .entry(location.symbol_id.clone())
                .or_default()
                .push(location.clone());
        }
        for (symbol_id, family) in &relationships {
            self.override_families
                .insert(symbol_id.clone(), family.clone());
        }
        self.document_override_families
            .insert(document.clone(), relationships);
        self.documents.insert(document, locations);
    }

    pub fn remove_document(&mut self, document: &str) {
        let Some(locations) = self.documents.remove(document) else {
            self.document_override_families.remove(document);
            return;
        };
        self.document_override_families.remove(document);
        for location in locations {
            let index = if location.role == "declaration" {
                &mut self.definitions
            } else {
                &mut self.references
            };
            if let Some(entries) = index.get_mut(&location.symbol_id) {
                entries.retain(|entry| entry.document != document);
                if entries.is_empty() {
                    index.remove(&location.symbol_id);
                }
            }
        }
        self.rebuild_override_families();
    }

    pub fn definitions(&self, qualified_name: &str) -> &[SymbolLocation] {
        self.definitions
            .get(qualified_name)
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    pub fn references(&self, qualified_name: &str) -> &[SymbolLocation] {
        self.references
            .get(qualified_name)
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    pub fn declarations_named(&self, name: &str) -> Vec<SymbolLocation> {
        self.definitions
            .values()
            .flatten()
            .filter(|location| location.name == name)
            .cloned()
            .collect()
    }

    pub fn related_symbol_ids(&self, symbol_id: &str) -> Vec<String> {
        let Some(family) = self.override_families.get(symbol_id) else {
            return vec![symbol_id.to_owned()];
        };
        let mut related: Vec<_> = self
            .override_families
            .iter()
            .filter_map(|(candidate, candidate_family)| {
                (candidate_family == family).then_some(candidate.clone())
            })
            .collect();
        related.sort();
        related.dedup();
        related
    }

    pub fn override_family(&self, symbol_id: &str) -> Option<&str> {
        self.override_families.get(symbol_id).map(String::as_str)
    }

    pub fn incoming_calls(&self, symbol_id: &str) -> Vec<SymbolLocation> {
        self.documents
            .values()
            .flatten()
            .filter(|location| location.role == "call_edge" && location.symbol_id == symbol_id)
            .cloned()
            .collect()
    }

    pub fn outgoing_calls(&self, symbol_id: &str) -> Vec<SymbolLocation> {
        self.documents
            .values()
            .flatten()
            .filter(|location| location.role == "call_edge" && location.qualified_name == symbol_id)
            .cloned()
            .collect()
    }

    pub fn document_call_edges(&self, document: &str) -> Vec<SymbolLocation> {
        self.documents
            .get(document)
            .into_iter()
            .flatten()
            .filter(|location| location.role == "call_edge")
            .cloned()
            .collect()
    }

    pub fn direct_supertypes(&self, symbol_id: &str) -> Vec<SymbolLocation> {
        self.documents
            .values()
            .flatten()
            .filter(|location| location.role == "type_edge" && location.qualified_name == symbol_id)
            .cloned()
            .collect()
    }

    pub fn direct_subtypes(&self, symbol_id: &str) -> Vec<SymbolLocation> {
        self.documents
            .values()
            .flatten()
            .filter(|location| location.role == "type_edge" && location.symbol_id == symbol_id)
            .cloned()
            .collect()
    }

    fn rebuild_override_families(&mut self) {
        self.override_families.clear();
        for relationships in self.document_override_families.values() {
            for (symbol_id, family) in relationships {
                self.override_families
                    .insert(symbol_id.clone(), family.clone());
            }
        }
    }

    pub fn symbol_at(&self, document: &str, offset: u64) -> Option<&SymbolLocation> {
        self.documents
            .get(document)?
            .iter()
            .filter(|location| location.start <= offset && offset <= location.end)
            .min_by_key(|location| location.end.saturating_sub(location.start))
    }

    pub fn rename_locations(&self, qualified_name: &str) -> Vec<SymbolLocation> {
        self.definitions(qualified_name)
            .iter()
            .chain(self.references(qualified_name))
            .cloned()
            .collect()
    }

    pub fn prepare_rename(
        &self,
        qualified_name: &str,
        new_name: &str,
    ) -> Result<Vec<RenameEdit>, RenameError> {
        if !is_java_identifier(new_name) {
            return Err(RenameError::InvalidIdentifier);
        }
        match self.definitions(qualified_name).len() {
            0 => return Err(RenameError::DefinitionNotFound),
            1 => {}
            _ => return Err(RenameError::AmbiguousDefinition),
        }
        let mut locations = self.rename_locations(qualified_name);
        locations.sort_by(|left, right| {
            left.document
                .cmp(&right.document)
                .then_with(|| left.start.cmp(&right.start))
                .then_with(|| left.end.cmp(&right.end))
        });
        locations.dedup_by(|left, right| {
            left.document == right.document && left.start == right.start && left.end == right.end
        });
        Ok(locations
            .into_iter()
            .map(|location| RenameEdit {
                document: location.document,
                start: location.start,
                end: location.end,
                new_text: new_name.to_owned(),
            })
            .collect())
    }

    pub fn workspace_symbols(&self, query: &str, limit: usize) -> Vec<SymbolLocation> {
        let query = query.to_lowercase();
        let mut matches: Vec<_> = self
            .definitions
            .values()
            .flatten()
            .filter_map(|location| {
                match_score(&location.name.to_lowercase(), &query)
                    .map(|score| (score, location.clone()))
            })
            .collect();
        matches.sort_by(|(left_score, left), (right_score, right)| {
            left_score
                .cmp(right_score)
                .then_with(|| left.name.cmp(&right.name))
                .then_with(|| left.document.cmp(&right.document))
                .then_with(|| left.start.cmp(&right.start))
        });
        matches
            .into_iter()
            .take(limit)
            .map(|(_, location)| location)
            .collect()
    }

    pub fn document_symbols(&self, document: &str) -> Vec<SymbolLocation> {
        let mut symbols: Vec<_> = self
            .documents
            .get(document)
            .into_iter()
            .flatten()
            .filter(|location| location.role == "declaration")
            .cloned()
            .collect();
        symbols.sort_by_key(|location| (location.start, location.end));
        symbols
    }

    pub fn completion_symbols(&self, query: &str, limit: usize) -> Vec<SymbolLocation> {
        let query = query.to_lowercase();
        let mut matches: Vec<_> = self
            .definitions
            .values()
            .chain(self.references.values())
            .flatten()
            .filter_map(|location| {
                match_score(&location.name.to_lowercase(), &query)
                    .map(|score| (score, location.clone()))
            })
            .collect();
        matches.sort_by(|(left_score, left), (right_score, right)| {
            left_score
                .cmp(right_score)
                .then_with(|| left.name.cmp(&right.name))
                .then_with(|| left.qualified_name.cmp(&right.qualified_name))
        });
        matches.dedup_by(|(_, left), (_, right)| {
            left.qualified_name == right.qualified_name && left.kind == right.kind
        });
        matches
            .into_iter()
            .take(limit)
            .map(|(_, location)| location)
            .collect()
    }
}

pub fn is_java_identifier(value: &str) -> bool {
    const KEYWORDS: &[&str] = &[
        "_",
        "abstract",
        "assert",
        "boolean",
        "break",
        "byte",
        "case",
        "catch",
        "char",
        "class",
        "const",
        "continue",
        "default",
        "do",
        "double",
        "else",
        "enum",
        "extends",
        "false",
        "final",
        "finally",
        "float",
        "for",
        "goto",
        "if",
        "implements",
        "import",
        "instanceof",
        "int",
        "interface",
        "long",
        "native",
        "new",
        "null",
        "package",
        "private",
        "protected",
        "public",
        "record",
        "return",
        "sealed",
        "short",
        "static",
        "strictfp",
        "super",
        "switch",
        "synchronized",
        "this",
        "throw",
        "throws",
        "transient",
        "true",
        "try",
        "var",
        "void",
        "volatile",
        "while",
        "yield",
    ];
    if KEYWORDS.contains(&value) {
        return false;
    }
    let mut characters = value.chars();
    let Some(first) = characters.next() else {
        return false;
    };
    (first == '_' || first == '$' || first.is_alphabetic())
        && characters
            .all(|character| character == '_' || character == '$' || character.is_alphanumeric())
}

fn match_score(candidate: &str, query: &str) -> Option<(u8, usize)> {
    if query.is_empty() {
        return Some((3, candidate.len()));
    }
    if candidate == query {
        return Some((0, 0));
    }
    if candidate.starts_with(query) {
        return Some((1, candidate.len() - query.len()));
    }
    if candidate.contains(query) {
        return Some((2, candidate.len() - query.len()));
    }
    let mut characters = candidate.char_indices();
    let mut first = None;
    let mut last = 0;
    for expected in query.chars() {
        let (index, _) = characters.find(|(_, character)| *character == expected)?;
        first.get_or_insert(index);
        last = index;
    }
    Some((3, last - first.unwrap_or(0)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SemanticSymbol;

    fn result(symbols: &[(&str, &str, &str)]) -> SemanticResult {
        SemanticResult {
            package_name: "demo".to_owned(),
            symbols: symbols
                .iter()
                .enumerate()
                .map(|(index, (role, name, qualified_name))| SemanticSymbol {
                    role: (*role).to_owned(),
                    kind: "class".to_owned(),
                    name: (*name).to_owned(),
                    qualified_name: (*qualified_name).to_owned(),
                    symbol_id: (*qualified_name).to_owned(),
                    start: index as u64,
                    end: index as u64 + 1,
                })
                .collect(),
            diagnostics: Vec::new(),
        }
    }

    #[test]
    fn indexes_definitions_references_and_replacements() {
        let mut index = SymbolIndex::default();
        index.update_document(
            "Model.java",
            &result(&[("declaration", "Model", "demo.Model")]),
        );
        index.update_document(
            "Service.java",
            &result(&[
                ("declaration", "Service", "demo.Service"),
                ("reference", "Model", "demo.Model"),
            ]),
        );

        assert_eq!(index.definitions("demo.Model").len(), 1);
        assert_eq!(index.references("demo.Model").len(), 1);
        assert_eq!(index.rename_locations("demo.Model").len(), 2);

        index.update_document(
            "Service.java",
            &result(&[("declaration", "Service", "demo.Service")]),
        );
        assert!(index.references("demo.Model").is_empty());
        assert_eq!(index.definitions("demo.Service").len(), 1);
    }

    #[test]
    fn workspace_search_prioritizes_exact_prefix_and_subsequence_matches() {
        let mut index = SymbolIndex::default();
        index.update_document(
            "Types.java",
            &result(&[
                ("declaration", "Pet", "demo.Pet"),
                ("declaration", "PetClinic", "demo.PetClinic"),
                ("declaration", "PersistentEntity", "demo.PersistentEntity"),
            ]),
        );

        let names: Vec<_> = index
            .workspace_symbols("pet", 10)
            .into_iter()
            .map(|location| location.name)
            .collect();
        assert_eq!(names, ["Pet", "PetClinic", "PersistentEntity"]);
    }

    #[test]
    fn completion_catalog_includes_external_references_without_fake_definitions() {
        let mut index = SymbolIndex::default();
        index.update_document(
            "Use.java",
            &result(&[
                ("declaration", "Use", "demo.Use"),
                ("reference", "ArrayList", "java.util.ArrayList"),
            ]),
        );

        assert!(index.workspace_symbols("Array", 10).is_empty());
        assert_eq!(
            index.completion_symbols("Array", 10)[0].qualified_name,
            "java.util.ArrayList"
        );
    }

    #[test]
    fn indexes_overloads_by_canonical_jvm_identity() {
        let mut index = SymbolIndex::default();
        let symbols = [
            ("declaration", "(I)V", 0, 3),
            ("declaration", "(Ljava/lang/String;)V", 4, 7),
            ("reference", "(I)V", 8, 11),
        ]
        .into_iter()
        .map(|(role, descriptor, start, end)| SemanticSymbol {
            role: role.to_owned(),
            kind: "method".to_owned(),
            name: "set".to_owned(),
            qualified_name: "demo.Box#set".to_owned(),
            symbol_id: format!("<unnamed>|demo/Box#set{descriptor}"),
            start,
            end,
        })
        .collect();
        index.update_document(
            "file:///Box.java",
            &SemanticResult {
                package_name: "demo".to_owned(),
                symbols,
                diagnostics: Vec::new(),
            },
        );

        assert_eq!(index.definitions("<unnamed>|demo/Box#set(I)V").len(), 1);
        assert_eq!(index.references("<unnamed>|demo/Box#set(I)V").len(), 1);
        assert_eq!(
            index
                .references("<unnamed>|demo/Box#set(Ljava/lang/String;)V")
                .len(),
            0
        );
    }

    #[test]
    fn rename_is_validated_deduplicated_and_definition_backed() {
        let mut index = SymbolIndex::default();
        index.update_document(
            "Model.java",
            &result(&[("declaration", "Model", "demo.Model")]),
        );
        let mut service = result(&[
            ("reference", "Model", "demo.Model"),
            ("reference", "Model", "demo.Model"),
        ]);
        service.symbols[1].start = service.symbols[0].start;
        service.symbols[1].end = service.symbols[0].end;
        index.update_document("Service.java", &service);

        let edits = index.prepare_rename("demo.Model", "Entity").unwrap();
        assert_eq!(edits.len(), 2);
        assert!(edits.iter().all(|edit| edit.new_text == "Entity"));
        assert_eq!(
            index.prepare_rename("demo.Model", "class"),
            Err(RenameError::InvalidIdentifier)
        );
        assert_eq!(
            index.prepare_rename("java.lang.String", "Text"),
            Err(RenameError::DefinitionNotFound)
        );
    }

    #[test]
    fn declaration_name_lookup_supports_rename_conflict_checks() {
        let mut index = SymbolIndex::default();
        index.update_document(
            "file:///Demo.java",
            &result(&[
                ("declaration", "first", "demo.Demo#first"),
                ("declaration", "second", "demo.Demo#second"),
                ("reference", "second", "demo.Demo#second"),
            ]),
        );
        let declarations = index.declarations_named("second");
        assert_eq!(declarations.len(), 1);
        assert_eq!(declarations[0].role, "declaration");
        assert_eq!(declarations[0].start, 1);
    }

    #[test]
    fn override_families_connect_declarations_calls_and_method_references() {
        let mut index = SymbolIndex::default();
        let family = "<unnamed>|demo/Service#value()Ljava/lang/String;";
        let result = SemanticResult {
            package_name: "demo".to_owned(),
            diagnostics: Vec::new(),
            symbols: [
                ("declaration", "demo.Service#value", family),
                ("override_family", family, family),
                (
                    "declaration",
                    "demo.Child#value",
                    "<unnamed>|demo/Child#value()Ljava/lang/String;",
                ),
                (
                    "override_family",
                    family,
                    "<unnamed>|demo/Child#value()Ljava/lang/String;",
                ),
            ]
            .into_iter()
            .enumerate()
            .map(
                |(index, (role, qualified_name, symbol_id))| SemanticSymbol {
                    role: role.to_owned(),
                    kind: "method".to_owned(),
                    name: "value".to_owned(),
                    qualified_name: qualified_name.to_owned(),
                    symbol_id: symbol_id.to_owned(),
                    start: index as u64,
                    end: index as u64 + 1,
                },
            )
            .collect(),
        };
        index.update_document("file:///Demo.java", &result);
        assert_eq!(
            index.related_symbol_ids(family),
            vec![
                "<unnamed>|demo/Child#value()Ljava/lang/String;".to_owned(),
                family.to_owned()
            ]
        );
    }
}
