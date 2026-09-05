use chrono::Utc;
use hex;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use tree_sitter::{Language, Node, Parser};
use uuid::Uuid;

use crate::store::SqliteStore;
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
    pub content_hash: String,
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

    /// Computes deterministic Blake3 content hash of normalized symbol body.
    pub fn blake3_content_hash(content: &str) -> String {
        let mut hasher = blake3::Hasher::new();
        for line in content.lines() {
            hasher.update(line.trim_end().as_bytes());
            hasher.update(b"\n");
        }
        hasher.finalize().to_hex().to_string()
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
        parser.set_language(&ts_lang).map_err(|e| {
            StrataError::Internal(format!("Failed to set tree-sitter language: {e}"))
        })?;

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
                            content_hash: Self::blake3_content_hash(raw_code),
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
                            content_hash: Self::blake3_content_hash(raw_code),
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
                            content_hash: Self::blake3_content_hash(raw_code),
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
                            content_hash: Self::blake3_content_hash(raw_code),
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
                            content_hash: Self::blake3_content_hash(raw_code),
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
                            content_hash: Self::blake3_content_hash(raw_code),
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
                                        content_hash: Self::blake3_content_hash(raw_code),
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
                            content_hash: Self::blake3_content_hash(raw_code),
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
                            content_hash: Self::blake3_content_hash(raw_code),
                            doc_comment: None,
                        });

                        if let Some(body) = target_node.child_by_field_name("body") {
                            let mut body_cursor = body.walk();
                            for item in body.children(&mut body_cursor) {
                                if item.kind() == "method_definition" {
                                    if let Some(method_name_node) = item.child_by_field_name("name")
                                    {
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
                                            content_hash: Self::blake3_content_hash(method_raw),
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
                            content_hash: Self::blake3_content_hash(raw_code),
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
                            content_hash: Self::blake3_content_hash(raw_code),
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
                            content_hash: Self::blake3_content_hash(raw_code),
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
                            content_hash: Self::blake3_content_hash(raw_code),
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
                            content_hash: Self::blake3_content_hash(raw_code),
                            doc_comment: None,
                        });

                        if let Some(body) = child.child_by_field_name("body") {
                            let mut body_cursor = body.walk();
                            for item in body.children(&mut body_cursor) {
                                if item.kind() == "function_definition" {
                                    if let Some(method_name_node) = item.child_by_field_name("name")
                                    {
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
                                            content_hash: Self::blake3_content_hash(method_raw),
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
    pub stale_facts: Vec<Uuid>,
    pub suspicious_facts: Vec<Uuid>,
    pub moved_anchors: Vec<Uuid>,
    pub invalidated_facts: Vec<Uuid>,
    pub updated_facts: Vec<Uuid>,
    pub active_facts: Vec<Uuid>,
    pub total_facts_scanned: usize,
    #[serde(default)]
    pub merkle_root_before: Option<String>,
    #[serde(default)]
    pub merkle_root_after: Option<String>,
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

    /// Creates a CodeAnchor for an extracted symbol with optional Git commit hash and Blake3 content hash.
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
        anchor.content_hash = Some(symbol.content_hash.clone());
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
                            report.stale_facts.push(fact.id);
                        } else {
                            report.active_facts.push(fact.id);
                        }
                    } else {
                        // Symbol deleted -> Invalidate anchor & deprecate fact
                        anchor.invalidate();
                        fact.status = FactStatus::Deprecated;
                        fact.last_updated_at = Utc::now();
                        report.invalidated_facts.push(fact.id);
                        report.stale_facts.push(fact.id);
                    }
                } else if anchor.is_valid {
                    report.active_facts.push(fact.id);
                }
            }
        }

        Ok(report)
    }

    /// Reconciles all code-anchored semantic facts in the SQLite store against the full workspace source files.
    ///
    /// Lifecycle execution:
    /// 1. Merkle / AST comparison between anchored symbol version and current workspace ASTs.
    /// 2. Fallback Blake3 (content-hash) matching to tolerate renames and file moves without invalidating facts if symbol body is identical.
    /// 3. Direct transition of modified/deleted facts to `FactStatus::Stale` with anchor invalidation.
    /// 4. Propagation of `FactStatus::Suspicious` to dependent facts via the Causal World Model blast radius.
    /// 5. Persistent atomic update in SQLite store.
    pub async fn reconcile_workspace_on_commit(
        &self,
        store: &SqliteStore,
        workspace_files: &[(&str, &str)], // (relative_file_path, file_content)
        git_commit_hash: Option<&str>,
        world_model: Option<&strata_reasoning::WorldModel>,
    ) -> Result<ReconciliationReport, StrataError> {
        let mut exact_map: HashMap<(String, String), ExtractedSymbol> = HashMap::new();
        let mut blake3_map: HashMap<String, Vec<(String, ExtractedSymbol)>> = HashMap::new();
        let mut all_current_symbols: Vec<ExtractedSymbol> = Vec::new();

        for &(path, content) in workspace_files {
            let normalized_path = path.replace('\\', "/");
            if let Ok(symbols) = self.parser.parse_file(&normalized_path, content) {
                for sym in symbols {
                    exact_map.insert(
                        (normalized_path.clone(), sym.symbol_path.clone()),
                        sym.clone(),
                    );
                    blake3_map
                        .entry(sym.content_hash.clone())
                        .or_default()
                        .push((normalized_path.clone(), sym.clone()));
                    // Also index by node_hash for legacy fallback
                    blake3_map
                        .entry(sym.node_hash.clone())
                        .or_default()
                        .push((normalized_path.clone(), sym.clone()));
                    all_current_symbols.push(sym);
                }
            }
        }

        let merkle_root_after = Self::compute_merkle_tree_hash(&all_current_symbols);

        let mut facts = store.get_all_semantic_facts(None, None, 10000)?;
        let mut report = ReconciliationReport {
            total_facts_scanned: facts.len(),
            merkle_root_after: Some(merkle_root_after),
            ..Default::default()
        };

        let mut stale_targets: Vec<(Uuid, String, String)> = Vec::new();
        let mut modified_fact_ids: HashSet<Uuid> = HashSet::new();

        // 1. First pass: Reconcile code anchors
        for fact in facts.iter_mut() {
            if fact.code_anchor.is_none() {
                continue;
            }

            let (is_valid, norm_anchor_path, symbol_path, ast_node_hash, content_hash) = {
                let a = fact.code_anchor.as_ref().unwrap();
                (
                    a.is_valid,
                    a.file_path.replace('\\', "/"),
                    a.symbol_path.clone(),
                    a.ast_node_hash.clone(),
                    a.content_hash.clone(),
                )
            };

            if !is_valid
                && (fact.status == FactStatus::Deprecated || fact.status == FactStatus::Stale)
            {
                continue;
            }

            let lookup_key = (norm_anchor_path.clone(), symbol_path.clone());
            if let Some(current_sym) = exact_map.get(&lookup_key) {
                // Exact location matched!
                if ast_node_hash == current_sym.node_hash
                    || content_hash.as_deref() == Some(&current_sym.content_hash)
                {
                    // Symbol is intact & active
                    let anchor = fact.code_anchor.as_mut().unwrap();
                    let mut updated = false;
                    if anchor.start_line != current_sym.start_line
                        || anchor.end_line != current_sym.end_line
                    {
                        anchor.start_line = current_sym.start_line;
                        anchor.end_line = current_sym.end_line;
                        updated = true;
                    }
                    if anchor.content_hash.is_none() {
                        anchor.content_hash = Some(current_sym.content_hash.clone());
                        updated = true;
                    }
                    if let Some(commit) = git_commit_hash {
                        if anchor.git_commit_hash.as_deref() != Some(commit) {
                            anchor.git_commit_hash = Some(commit.to_string());
                            updated = true;
                        }
                    }
                    if updated {
                        fact.last_updated_at = Utc::now();
                        report.updated_facts.push(fact.id);
                        modified_fact_ids.insert(fact.id);
                    }
                    report.active_facts.push(fact.id);
                } else {
                    // Symbol was modified in place -> Transmit to Stale
                    fact.mark_stale();
                    report.stale_facts.push(fact.id);
                    report.invalidated_facts.push(fact.id);
                    stale_targets.push((fact.id, norm_anchor_path.clone(), symbol_path.clone()));
                    modified_fact_ids.insert(fact.id);
                }
            } else {
                // Symbol NOT found at (file_path, symbol_path)
                // Fallback: Check Blake3 content-hash across entire workspace to tolerate renames/moves
                let target_hash = content_hash.as_deref().unwrap_or(&ast_node_hash);

                if let Some(matches) = blake3_map.get(target_hash) {
                    if let Some((new_file, new_sym)) = matches.first() {
                        // Found relocated symbol with identical content body!
                        let anchor = fact.code_anchor.as_mut().unwrap();
                        anchor.file_path = new_file.clone();
                        anchor.symbol_path = new_sym.symbol_path.clone();
                        anchor.symbol_type = new_sym.symbol_type;
                        anchor.start_line = new_sym.start_line;
                        anchor.end_line = new_sym.end_line;
                        anchor.ast_node_hash = new_sym.node_hash.clone();
                        anchor.content_hash = Some(new_sym.content_hash.clone());
                        if let Some(commit) = git_commit_hash {
                            anchor.git_commit_hash = Some(commit.to_string());
                        }
                        anchor.is_valid = true;
                        anchor.valid_until = None;
                        fact.last_updated_at = Utc::now();
                        report.moved_anchors.push(fact.id);
                        report.active_facts.push(fact.id);
                        report.updated_facts.push(fact.id);
                        modified_fact_ids.insert(fact.id);
                    } else {
                        fact.mark_stale();
                        report.stale_facts.push(fact.id);
                        report.invalidated_facts.push(fact.id);
                        stale_targets.push((
                            fact.id,
                            norm_anchor_path.clone(),
                            symbol_path.clone(),
                        ));
                        modified_fact_ids.insert(fact.id);
                    }
                } else {
                    // Symbol truly deleted or changed beyond recognition
                    fact.mark_stale();
                    report.stale_facts.push(fact.id);
                    report.invalidated_facts.push(fact.id);
                    stale_targets.push((fact.id, norm_anchor_path.clone(), symbol_path.clone()));
                    modified_fact_ids.insert(fact.id);
                }
            }
        }

        // 2. Second pass: Causal blast radius propagation for suspicious facts
        if !stale_targets.is_empty() {
            let mut impacted_identifiers: HashSet<String> = HashSet::new();

            for (_id, stale_file, stale_sym) in &stale_targets {
                impacted_identifiers.insert(stale_file.clone());
                impacted_identifiers.insert(stale_sym.clone());

                if let Some(filename) = std::path::Path::new(stale_file)
                    .file_name()
                    .and_then(|f| f.to_str())
                {
                    impacted_identifiers.insert(filename.to_string());
                }

                if let Some(wm) = world_model {
                    if let Ok(blast) = wm.predict_impact(stale_file, 3).await {
                        for imp in blast.direct_impacts.iter().chain(&blast.transitive_impacts) {
                            impacted_identifiers.insert(imp.name.clone());
                            impacted_identifiers.insert(imp.node_id.clone());
                            if let Some(ref p) = imp.path {
                                impacted_identifiers.insert(p.clone());
                            }
                        }
                    }
                }
            }

            for fact in facts.iter_mut() {
                if fact.status == FactStatus::Active {
                    let mut is_dependent = false;

                    if let Some(ref anc) = fact.code_anchor {
                        let anc_norm = anc.file_path.replace('\\', "/");
                        if impacted_identifiers.contains(&anc_norm)
                            || impacted_identifiers.contains(&anc.symbol_path)
                        {
                            is_dependent = true;
                        }
                    }

                    for ev in &fact.evidence {
                        let ev_norm = ev.source_id.replace('\\', "/");
                        if impacted_identifiers.contains(&ev_norm) {
                            is_dependent = true;
                        }
                    }

                    for id_str in &impacted_identifiers {
                        if !id_str.is_empty()
                            && (fact.statement.contains(id_str) || fact.tags.contains(id_str))
                        {
                            is_dependent = true;
                            break;
                        }
                    }

                    if is_dependent {
                        fact.mark_suspicious();
                        report.suspicious_facts.push(fact.id);
                        report.active_facts.retain(|&id| id != fact.id);
                        modified_fact_ids.insert(fact.id);
                    }
                }
            }
        }

        // 3. Third pass: Atomic persistence to SQLite
        for fact in &facts {
            if modified_fact_ids.contains(&fact.id) {
                store.insert_or_update_semantic_fact(fact)?;
            }
        }

        Ok(report)
    }

    /// Recursively scans a workspace directory on disk and reconciles all semantic facts.
    pub async fn reconcile_workspace_dir(
        &self,
        store: &SqliteStore,
        workspace_root: &Path,
        git_commit_hash: Option<&str>,
        world_model: Option<&strata_reasoning::WorldModel>,
    ) -> Result<ReconciliationReport, StrataError> {
        let mut paths: Vec<PathBuf> = Vec::new();
        let mut contents: Vec<String> = Vec::new();

        self.collect_workspace_files(workspace_root, workspace_root, &mut paths, &mut contents)?;

        let mut relative_paths: Vec<String> = Vec::new();
        for p in &paths {
            let rel = p
                .strip_prefix(workspace_root)
                .unwrap_or(p)
                .to_string_lossy()
                .replace('\\', "/");
            relative_paths.push(rel);
        }

        let file_slices: Vec<(&str, &str)> = relative_paths
            .iter()
            .zip(contents.iter())
            .map(|(p, c)| (p.as_str(), c.as_str()))
            .collect();

        self.reconcile_workspace_on_commit(store, &file_slices, git_commit_hash, world_model)
            .await
    }

    fn collect_workspace_files(
        &self,
        current_dir: &Path,
        _root_dir: &Path,
        out_paths: &mut Vec<PathBuf>,
        out_contents: &mut Vec<String>,
    ) -> Result<(), StrataError> {
        if !current_dir.exists() || !current_dir.is_dir() {
            return Ok(());
        }

        let entries = std::fs::read_dir(current_dir).map_err(|e| {
            StrataError::Internal(format!("Failed to read dir {}: {e}", current_dir.display()))
        })?;

        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();

            // Skip hidden and build directories
            if name.starts_with('.')
                || name == "target"
                || name == "node_modules"
                || name == "dist"
                || name == "build"
                || name == "venv"
                || name == ".venv"
            {
                continue;
            }

            if path.is_dir() {
                self.collect_workspace_files(&path, _root_dir, out_paths, out_contents)?;
            } else if path.is_file() {
                let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
                match ext {
                    "rs" | "ts" | "tsx" | "js" | "jsx" | "py" | "pyi" => {
                        if let Ok(content) = std::fs::read_to_string(&path) {
                            out_paths.push(path);
                            out_contents.push(content);
                        }
                    }
                    _ => {}
                }
            }
        }

        Ok(())
    }
}
