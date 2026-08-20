use std::collections::HashMap;
use chrono::Utc;
use hex;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tree_sitter::{Language, Node, Parser};
use uuid::Uuid;

use strata_core::errors::StrataError;
use strata_core::schemas::{CodeAnchor, FactStatus, SemanticFact, SymbolType};

/// Supported programming languages for structural AST indexing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LanguageKind {
    Rust,
    TypeScript,
    JavaScript,
    Python,
    Unknown,
}

impl LanguageKind {
    pub fn from_file_path(path: &str) -> Self {
        let p = std::path::Path::new(path);
        match p.extension().and_then(|ext| ext.to_str()).unwrap_or("") {
            "rs" => LanguageKind::Rust,
            "ts" | "tsx" | "mts" | "cts" => LanguageKind::TypeScript,
            "js" | "jsx" | "mjs" | "cjs" => LanguageKind::JavaScript,
            "py" | "pyi" => LanguageKind::Python,
            _ => LanguageKind::Unknown,
        }
    }

    pub fn tree_sitter_language(&self) -> Option<Language> {
        match self {
            LanguageKind::Rust => Some(tree_sitter_rust::LANGUAGE.into()),
            LanguageKind::TypeScript => Some(tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()),
            LanguageKind::JavaScript => Some(tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()),
            LanguageKind::Python => Some(tree_sitter_python::LANGUAGE.into()),
            LanguageKind::Unknown => None,
        }
    }
}

/// Extracted code symbol representation from Tree-Sitter AST.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExtractedSymbol {
    pub name: String,
    pub symbol_path: String,
    pub symbol_type: SymbolType,
    pub start_line: u32,
    pub end_line: u32,
    pub raw_code: String,
    pub node_hash: String,
    #[serde(default)]
    pub doc_comment: Option<String>,
}

/// AST Parser utilizing Tree-Sitter for multi-language code analysis.
pub struct AstParser;

impl Default for AstParser {
    fn default() -> Self {
        Self::new()
    }
}

impl AstParser {
    pub fn new() -> Self {
        Self
    }

    /// Computes deterministic SHA-256 hash of normalized code content.
    pub fn hash_content(content: &str) -> String {
        let mut hasher = Sha256::new();
        // Normalize line breaks to \n for deterministic cross-platform hashes
        for line in content.lines() {
            hasher.update(line.trim_end().as_bytes());
            hasher.update(b"\n");
        }
        hex::encode(hasher.finalize())
    }

    /// Parses source code and returns all extracted structural symbols.
    pub fn parse_source(
        &self,
        source_code: &str,
        lang: LanguageKind,
        file_path_hint: Option<&str>,
    ) -> Result<Vec<ExtractedSymbol>, StrataError> {
        let ts_lang = lang.tree_sitter_language().ok_or_else(|| {
            StrataError::Validation(format!("Unsupported language for AST parsing: {:?}", lang))
        })?;

        let mut parser = Parser::new();
        parser
            .set_language(&ts_lang)
            .map_err(|e| StrataError::Internal(format!("Failed to set tree-sitter language: {e}")))?;

        let tree = parser
            .parse(source_code, None)
            .ok_or_else(|| StrataError::Internal("Tree-sitter parse failed".to_string()))?;

        let mut symbols = Vec::new();
        let prefix = file_path_hint.unwrap_or("");

        match lang {
            LanguageKind::Rust => {
                self.extract_rust_symbols(tree.root_node(), source_code, prefix, &mut symbols);
            }
            LanguageKind::TypeScript | LanguageKind::JavaScript => {
                self.extract_ts_symbols(tree.root_node(), source_code, prefix, &mut symbols);
            }
            LanguageKind::Python => {
                self.extract_python_symbols(tree.root_node(), source_code, prefix, &mut symbols);
            }
            LanguageKind::Unknown => {}
        }

        Ok(symbols)
    }

    /// Parses a file by path and source code.
    pub fn parse_file(
        &self,
        file_path: &str,
        source_code: &str,
    ) -> Result<Vec<ExtractedSymbol>, StrataError> {
        let lang = LanguageKind::from_file_path(file_path);
        self.parse_source(source_code, lang, Some(file_path))
    }

    fn extract_rust_symbols(
        &self,
        root: Node,
        source: &str,
        _prefix: &str,
        out: &mut Vec<ExtractedSymbol>,
    ) {
        let mut cursor = root.walk();
        for child in root.children(&mut cursor) {
            match child.kind() {
                "function_item" => {
                    if let Some(name_node) = child.child_by_field_name("name") {
                        let name = &source[name_node.byte_range()];
                        let raw_code = &source[child.byte_range()];
                        out.push(ExtractedSymbol {
                            name: name.to_string(),
                            symbol_path: name.to_string(),
                            symbol_type: SymbolType::Function,
                            start_line: child.start_position().row as u32 + 1,
                            end_line: child.end_position().row as u32 + 1,
                            raw_code: raw_code.to_string(),
                            node_hash: Self::hash_content(raw_code),
                            doc_comment: None,
                        });
                    }
                }
                "struct_item" => {
                    if let Some(name_node) = child.child_by_field_name("name") {
                        let name = &source[name_node.byte_range()];
                        let raw_code = &source[child.byte_range()];
                        out.push(ExtractedSymbol {
                            name: name.to_string(),
                            symbol_path: name.to_string(),
                            symbol_type: SymbolType::Struct,
                            start_line: child.start_position().row as u32 + 1,
                            end_line: child.end_position().row as u32 + 1,
                            raw_code: raw_code.to_string(),
                            node_hash: Self::hash_content(raw_code),
                            doc_comment: None,
                        });
                    }
                }
                "enum_item" => {
                    if let Some(name_node) = child.child_by_field_name("name") {
                        let name = &source[name_node.byte_range()];
                        let raw_code = &source[child.byte_range()];
                        out.push(ExtractedSymbol {
                            name: name.to_string(),
                            symbol_path: name.to_string(),
                            symbol_type: SymbolType::Enum,
                            start_line: child.start_position().row as u32 + 1,
                            end_line: child.end_position().row as u32 + 1,
                            raw_code: raw_code.to_string(),
                            node_hash: Self::hash_content(raw_code),
                            doc_comment: None,
                        });
                    }
                }
                "trait_item" => {
                    if let Some(name_node) = child.child_by_field_name("name") {
                        let name = &source[name_node.byte_range()];
                        let raw_code = &source[child.byte_range()];
                        out.push(ExtractedSymbol {
                            name: name.to_string(),
                            symbol_path: name.to_string(),
                            symbol_type: SymbolType::Trait,
                            start_line: child.start_position().row as u32 + 1,
                            end_line: child.end_position().row as u32 + 1,
                            raw_code: raw_code.to_string(),
                            node_hash: Self::hash_content(raw_code),
                            doc_comment: None,
                        });
                    }
                }
                "type_item" => {
                    if let Some(name_node) = child.child_by_field_name("name") {
                        let name = &source[name_node.byte_range()];
                        let raw_code = &source[child.byte_range()];
                        out.push(ExtractedSymbol {
                            name: name.to_string(),
                            symbol_path: name.to_string(),
                            symbol_type: SymbolType::TypeAlias,
                            start_line: child.start_position().row as u32 + 1,
                            end_line: child.end_position().row as u32 + 1,
                            raw_code: raw_code.to_string(),
                            node_hash: Self::hash_content(raw_code),
                            doc_comment: None,
                        });
                    }
                }
                "mod_item" => {
                    if let Some(name_node) = child.child_by_field_name("name") {
                        let name = &source[name_node.byte_range()];
                        let raw_code = &source[child.byte_range()];
                        out.push(ExtractedSymbol {
                            name: name.to_string(),
                            symbol_path: name.to_string(),
                            symbol_type: SymbolType::Module,
                            start_line: child.start_position().row as u32 + 1,
                            end_line: child.end_position().row as u32 + 1,
                            raw_code: raw_code.to_string(),
                            node_hash: Self::hash_content(raw_code),
                            doc_comment: None,
                        });
                    }
                }
                "impl_item" => {
                    let impl_type_name = child
                        .child_by_field_name("type")
                        .map(|t| source[t.byte_range()].trim().to_string())
                        .unwrap_or_else(|| "Unknown".to_string());

                    let trait_name = child
                        .child_by_field_name("trait")
                        .map(|t| source[t.byte_range()].trim().to_string());

                    if let Some(body) = child.child_by_field_name("body") {
                        let mut body_cursor = body.walk();
                        for item in body.children(&mut body_cursor) {
                            if item.kind() == "function_item" {
                                if let Some(fn_name_node) = item.child_by_field_name("name") {
                                    let fn_name = &source[fn_name_node.byte_range()];
                                    let sym_path = if let Some(ref tr) = trait_name {
                                        format!("<{} as {}>::{}", impl_type_name, tr, fn_name)
                                    } else {
                                        format!("{}::{}", impl_type_name, fn_name)
                                    };
                                    let raw_code = &source[item.byte_range()];
                                    out.push(ExtractedSymbol {
                                        name: fn_name.to_string(),
                                        symbol_path: sym_path,
                                        symbol_type: SymbolType::Method,
                                        start_line: item.start_position().row as u32 + 1,
                                        end_line: item.end_position().row as u32 + 1,
                                        raw_code: raw_code.to_string(),
                                        node_hash: Self::hash_content(raw_code),
                                        doc_comment: None,
                                    });
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }

    fn extract_ts_symbols(
        &self,
        root: Node,
        source: &str,
        _prefix: &str,
        out: &mut Vec<ExtractedSymbol>,
    ) {
        let mut cursor = root.walk();
        for child in root.children(&mut cursor) {
            let target_node = if child.kind() == "export_statement" {
                if let Some(decl) = child.child_by_field_name("declaration") {
                    decl
                } else {
                    continue;
                }
            } else {
                child
            };

            match target_node.kind() {
                "function_declaration" | "generator_function_declaration" => {
                    if let Some(name_node) = target_node.child_by_field_name("name") {
                        let name = &source[name_node.byte_range()];
                        let raw_code = &source[target_node.byte_range()];
                        out.push(ExtractedSymbol {
                            name: name.to_string(),
                            symbol_path: name.to_string(),
                            symbol_type: SymbolType::Function,
                            start_line: target_node.start_position().row as u32 + 1,
                            end_line: target_node.end_position().row as u32 + 1,
                            raw_code: raw_code.to_string(),
                            node_hash: Self::hash_content(raw_code),
                            doc_comment: None,
                        });
                    }
                }
                "class_declaration" => {
                    if let Some(name_node) = target_node.child_by_field_name("name") {
                        let class_name = &source[name_node.byte_range()];
                        let raw_code = &source[target_node.byte_range()];
                        out.push(ExtractedSymbol {
                            name: class_name.to_string(),
                            symbol_path: class_name.to_string(),
                            symbol_type: SymbolType::Class,
                            start_line: target_node.start_position().row as u32 + 1,
                            end_line: target_node.end_position().row as u32 + 1,
                            raw_code: raw_code.to_string(),
                            node_hash: Self::hash_content(raw_code),
                            doc_comment: None,
                        });

                        if let Some(body) = target_node.child_by_field_name("body") {
                            let mut body_cursor = body.walk();
                            for item in body.children(&mut body_cursor) {
                                if item.kind() == "method_definition" {
                                    if let Some(method_name_node) = item.child_by_field_name("name") {
                                        let method_name = &source[method_name_node.byte_range()];
                                        let method_raw = &source[item.byte_range()];
                                        out.push(ExtractedSymbol {
                                            name: method_name.to_string(),
                                            symbol_path: format!("{}.{}", class_name, method_name),
                                            symbol_type: SymbolType::Method,
                                            start_line: item.start_position().row as u32 + 1,
                                            end_line: item.end_position().row as u32 + 1,
                                            raw_code: method_raw.to_string(),
                                            node_hash: Self::hash_content(method_raw),
                                            doc_comment: None,
                                        });
                                    }
                                }
                            }
                        }
                    }
                }
                "interface_declaration" => {
                    if let Some(name_node) = target_node.child_by_field_name("name") {
                        let name = &source[name_node.byte_range()];
                        let raw_code = &source[target_node.byte_range()];
                        out.push(ExtractedSymbol {
                            name: name.to_string(),
                            symbol_path: name.to_string(),
                            symbol_type: SymbolType::Interface,
                            start_line: target_node.start_position().row as u32 + 1,
                            end_line: target_node.end_position().row as u32 + 1,
                            raw_code: raw_code.to_string(),
                            node_hash: Self::hash_content(raw_code),
                            doc_comment: None,
                        });
                    }
                }
                "type_alias_declaration" => {
                    if let Some(name_node) = target_node.child_by_field_name("name") {
                        let name = &source[name_node.byte_range()];
                        let raw_code = &source[target_node.byte_range()];
                        out.push(ExtractedSymbol {
                            name: name.to_string(),
                            symbol_path: name.to_string(),
                            symbol_type: SymbolType::TypeAlias,
                            start_line: target_node.start_position().row as u32 + 1,
                            end_line: target_node.end_position().row as u32 + 1,
                            raw_code: raw_code.to_string(),
                            node_hash: Self::hash_content(raw_code),
                            doc_comment: None,
                        });
                    }
                }
                "enum_declaration" => {
                    if let Some(name_node) = target_node.child_by_field_name("name") {
                        let name = &source[name_node.byte_range()];
                        let raw_code = &source[target_node.byte_range()];
                        out.push(ExtractedSymbol {
                            name: name.to_string(),
                            symbol_path: name.to_string(),
                            symbol_type: SymbolType::Enum,
                            start_line: target_node.start_position().row as u32 + 1,
                            end_line: target_node.end_position().row as u32 + 1,
                            raw_code: raw_code.to_string(),
                            node_hash: Self::hash_content(raw_code),
                            doc_comment: None,
                        });
                    }
                }
                _ => {}
            }
        }
    }

    fn extract_python_symbols(
        &self,
        root: Node,
        source: &str,
        _prefix: &str,
        out: &mut Vec<ExtractedSymbol>,
    ) {
        let mut cursor = root.walk();
        for child in root.children(&mut cursor) {
            match child.kind() {
                "function_definition" => {
                    if let Some(name_node) = child.child_by_field_name("name") {
                        let name = &source[name_node.byte_range()];
                        let raw_code = &source[child.byte_range()];
                        out.push(ExtractedSymbol {
                            name: name.to_string(),
                            symbol_path: name.to_string(),
                            symbol_type: SymbolType::Function,
                            start_line: child.start_position().row as u32 + 1,
                            end_line: child.end_position().row as u32 + 1,
                            raw_code: raw_code.to_string(),
                            node_hash: Self::hash_content(raw_code),
                            doc_comment: None,
                        });
                    }
                }
                "class_definition" => {
                    if let Some(name_node) = child.child_by_field_name("name") {
                        let class_name = &source[name_node.byte_range()];
                        let raw_code = &source[child.byte_range()];
                        out.push(ExtractedSymbol {
                            name: class_name.to_string(),
                            symbol_path: class_name.to_string(),
                            symbol_type: SymbolType::Class,
                            start_line: child.start_position().row as u32 + 1,
                            end_line: child.end_position().row as u32 + 1,
                            raw_code: raw_code.to_string(),
                            node_hash: Self::hash_content(raw_code),
                            doc_comment: None,
                        });

                        if let Some(body) = child.child_by_field_name("body") {
                            let mut body_cursor = body.walk();
                            for item in body.children(&mut body_cursor) {
                                if item.kind() == "function_definition" {
                                    if let Some(method_name_node) = item.child_by_field_name("name") {
                                        let method_name = &source[method_name_node.byte_range()];
                                        let method_raw = &source[item.byte_range()];
                                        out.push(ExtractedSymbol {
                                            name: method_name.to_string(),
                                            symbol_path: format!("{}.{}", class_name, method_name),
                                            symbol_type: SymbolType::Method,
                                            start_line: item.start_position().row as u32 + 1,
                                            end_line: item.end_position().row as u32 + 1,
                                            raw_code: method_raw.to_string(),
                                            node_hash: Self::hash_content(method_raw),
                                            doc_comment: None,
                                        });
                                    }
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }
}

/// Difference result from comparing old anchors against newly parsed AST.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct AstDiffResult {
    pub unchanged: Vec<CodeAnchor>,
    pub modified: Vec<(CodeAnchor, CodeAnchor)>, // (old_anchor, new_anchor)
    pub deleted: Vec<CodeAnchor>,
    pub added: Vec<ExtractedSymbol>,
}

/// Report resulting from bi-temporal fact reconciliation against current source code.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct ReconciliationReport {
    pub invalidated_facts: Vec<Uuid>,
    pub updated_facts: Vec<Uuid>,
    pub active_facts: Vec<Uuid>,
}

/// Engine managing Git Merkle Tree calculations and CodeAnchor lifecycle.
pub struct CodeAnchorEngine {
    parser: AstParser,
}

impl Default for CodeAnchorEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl CodeAnchorEngine {
    pub fn new() -> Self {
        Self {
            parser: AstParser::new(),
        }
    }

    /// Creates a CodeAnchor for an extracted symbol with optional Git commit hash.
    pub fn create_anchor(
        &self,
        file_path: &str,
        symbol: &ExtractedSymbol,
        git_commit_hash: Option<&str>,
    ) -> CodeAnchor {
        let mut anchor = CodeAnchor::new(
            file_path,
            &symbol.symbol_path,
            symbol.symbol_type,
            &symbol.node_hash,
            symbol.start_line,
            symbol.end_line,
        );
        if let Some(commit) = git_commit_hash {
            anchor = anchor.with_git_commit(commit);
        }
        anchor
    }

    /// Computes a deterministic Merkle Root hash from a slice of extracted symbols.
    pub fn compute_merkle_tree_hash(symbols: &[ExtractedSymbol]) -> String {
        if symbols.is_empty() {
            return "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855".to_string();
        }

        let mut sorted_symbols = symbols.to_vec();
        sorted_symbols.sort_by(|a, b| a.symbol_path.cmp(&b.symbol_path));

        let mut current_hashes: Vec<String> = sorted_symbols
            .iter()
            .map(|s| {
                let mut h = Sha256::new();
                h.update(s.symbol_path.as_bytes());
                h.update(b":");
                h.update(s.node_hash.as_bytes());
                hex::encode(h.finalize())
            })
            .collect();

        while current_hashes.len() > 1 {
            let mut next_level = Vec::new();
            for chunk in current_hashes.chunks(2) {
                let mut h = Sha256::new();
                h.update(chunk[0].as_bytes());
                if chunk.len() > 1 {
                    h.update(chunk[1].as_bytes());
                } else {
                    h.update(chunk[0].as_bytes());
                }
                next_level.push(hex::encode(h.finalize()));
            }
            current_hashes = next_level;
        }

        current_hashes[0].clone()
    }

    /// Detects changes between previously recorded anchors and current source code.
    pub fn diff_anchors(
        &self,
        file_path: &str,
        old_anchors: &[CodeAnchor],
        current_source: &str,
        git_commit_hash: Option<&str>,
    ) -> Result<AstDiffResult, StrataError> {
        let current_symbols = self.parser.parse_file(file_path, current_source)?;
        let mut symbol_map: HashMap<String, ExtractedSymbol> = HashMap::new();
        for sym in current_symbols {
            symbol_map.insert(sym.symbol_path.clone(), sym);
        }

        let mut diff = AstDiffResult::default();
        let mut processed_symbols = HashMap::new();

        for old in old_anchors {
            if let Some(new_sym) = symbol_map.get(&old.symbol_path) {
                processed_symbols.insert(old.symbol_path.clone(), true);
                if old.ast_node_hash == new_sym.node_hash {
                    diff.unchanged.push(old.clone());
                } else {
                    let mut new_anchor = self.create_anchor(file_path, new_sym, git_commit_hash);
                    new_anchor.valid_from = Utc::now();
                    diff.modified.push((old.clone(), new_anchor));
                }
            } else {
                let mut deleted_anchor = old.clone();
                deleted_anchor.invalidate();
                diff.deleted.push(deleted_anchor);
            }
        }

        for (path, sym) in symbol_map {
            if !processed_symbols.contains_key(&path) {
                diff.added.push(sym);
            }
        }

        Ok(diff)
    }

    /// Reconciles a list of SemanticFacts bi-temporally against modified source code.
    /// If an anchored symbol's AST node hash changed or was deleted, the fact is deprecated/invalidated.
    pub fn reconcile_facts_bi_temporal(
        &self,
        facts: &mut [SemanticFact],
        current_source: &str,
        file_path: &str,
    ) -> Result<ReconciliationReport, StrataError> {
        let current_symbols = self.parser.parse_file(file_path, current_source)?;
        let symbol_map: HashMap<String, ExtractedSymbol> = current_symbols
            .into_iter()
            .map(|s| (s.symbol_path.clone(), s))
            .collect();

        let mut report = ReconciliationReport::default();

        for fact in facts.iter_mut() {
            if let Some(ref mut anchor) = fact.code_anchor {
                if anchor.file_path == file_path && anchor.is_valid {
                    if let Some(current_sym) = symbol_map.get(&anchor.symbol_path) {
                        if anchor.ast_node_hash != current_sym.node_hash {
                            // AST Node changed -> Invalidate anchor & deprecate fact
                            anchor.invalidate();
                            fact.status = FactStatus::Deprecated;
                            fact.last_updated_at = Utc::now();
                            report.invalidated_facts.push(fact.id);
                        } else {
                            report.active_facts.push(fact.id);
                        }
                    } else {
                        // Symbol deleted -> Invalidate anchor & deprecate fact
                        anchor.invalidate();
                        fact.status = FactStatus::Deprecated;
                        fact.last_updated_at = Utc::now();
                        report.invalidated_facts.push(fact.id);
                    }
                } else if anchor.is_valid {
                    report.active_facts.push(fact.id);
                }
            }
        }

        Ok(report)
    }
}
