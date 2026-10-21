use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use tree_sitter::{Node, Parser};
use uuid::Uuid;

use crate::ast::{AstParser, ExtractedSymbol, LanguageKind};
use crate::spreading_activation::EdgeKind;
use crate::store::{MemoryEdgeRecord, SqliteStore};
use strata_core::errors::StrataError;

/// Categorization of a call edge or dependency reference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CallType {
    /// Direct function invocation: `foo()`
    FunctionCall,
    /// Method invocation on an instance/struct: `obj.method()` or `self.method()`
    MethodCall,
    /// Constructor invocation: `Type::new()` or `new Class()`
    ConstructorCall,
    /// Module or symbol import: `use crate::foo`, `import { bar } from './baz'`
    Import,
    /// Macro invocation (e.g. in Rust `println!()`, `vec![]`)
    MacroCall,
}

impl std::fmt::Display for CallType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CallType::FunctionCall => write!(f, "function_call"),
            CallType::MethodCall => write!(f, "method_call"),
            CallType::ConstructorCall => write!(f, "constructor_call"),
            CallType::Import => write!(f, "import"),
            CallType::MacroCall => write!(f, "macro_call"),
        }
    }
}

impl std::str::FromStr for CallType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "function_call" | "function" => Ok(CallType::FunctionCall),
            "method_call" | "method" => Ok(CallType::MethodCall),
            "constructor_call" | "constructor" => Ok(CallType::ConstructorCall),
            "import" => Ok(CallType::Import),
            "macro_call" | "macro" => Ok(CallType::MacroCall),
            _ => Err(format!("Unknown call type: {s}")),
        }
    }
}

/// A directed edge in the code call graph representing a caller invoking or importing a callee.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallEdge {
    pub id: Uuid,
    pub caller_file: String,
    pub caller_symbol: String,
    pub callee_symbol: String,
    pub callee_file_hint: Option<String>,
    pub line_number: u32,
    pub call_type: CallType,
    pub created_at: DateTime<Utc>,
}

impl CallEdge {
    pub fn new(
        caller_file: impl Into<String>,
        caller_symbol: impl Into<String>,
        callee_symbol: impl Into<String>,
        line_number: u32,
        call_type: CallType,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            caller_file: caller_file.into(),
            caller_symbol: caller_symbol.into(),
            callee_symbol: callee_symbol.into(),
            callee_file_hint: None,
            line_number,
            call_type,
            created_at: Utc::now(),
        }
    }

    pub fn with_callee_file(mut self, file: impl Into<String>) -> Self {
        self.callee_file_hint = Some(file.into());
        self
    }
}

/// In-memory graph of call relationships and module dependencies.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct CallGraph {
    pub edges: Vec<CallEdge>,
}

impl CallGraph {
    pub fn new() -> Self {
        Self { edges: Vec::new() }
    }

    pub fn from_edges(edges: Vec<CallEdge>) -> Self {
        Self { edges }
    }

    pub fn add_edge(&mut self, edge: CallEdge) {
        self.edges.push(edge);
    }

    /// Finds all callers that invoke or reference the given callee symbol.
    pub fn callers_of(&self, callee_symbol: &str) -> Vec<&CallEdge> {
        self.edges
            .iter()
            .filter(|e| e.callee_symbol == callee_symbol)
            .collect()
    }

    /// Finds all callees invoked from the given caller symbol within a file.
    pub fn callees_of(&self, caller_file: &str, caller_symbol: &str) -> Vec<&CallEdge> {
        self.edges
            .iter()
            .filter(|e| e.caller_file == caller_file && e.caller_symbol == caller_symbol)
            .collect()
    }

    /// Returns all module/symbol import dependencies declared in a file.
    pub fn file_imports(&self, file_path: &str) -> Vec<&CallEdge> {
        self.edges
            .iter()
            .filter(|e| e.caller_file == file_path && e.call_type == CallType::Import)
            .collect()
    }

    /// Detects direct recursive self-calls or mutual call cycles.
    pub fn detect_recursive_calls(&self) -> Vec<(String, String)> {
        let mut recursive = Vec::new();
        for edge in &self.edges {
            if edge.caller_symbol == edge.callee_symbol && edge.caller_symbol != "<top-level>" {
                recursive.push((edge.caller_file.clone(), edge.caller_symbol.clone()));
            }
        }
        recursive
    }
}

/// Deterministic Native Call Graph Analyzer powered by Tree-Sitter in Rust.
pub struct CallGraphAnalyzer;

impl Default for CallGraphAnalyzer {
    fn default() -> Self {
        Self::new()
    }
}

impl CallGraphAnalyzer {
    pub fn new() -> Self {
        Self
    }

    /// Analyzes source code and extracts all call edges (function calls, method invocations, constructors, imports).
    pub fn analyze_source(
        &self,
        source_code: &str,
        lang: LanguageKind,
        file_path: &str,
    ) -> Result<Vec<CallEdge>, StrataError> {
        let ts_lang = lang.tree_sitter_language().ok_or_else(|| {
            StrataError::Validation(format!(
                "Unsupported language for call graph analysis: {:?}",
                lang
            ))
        })?;

        let mut parser = Parser::new();
        parser.set_language(&ts_lang).map_err(|e| {
            StrataError::Internal(format!("Failed to set tree-sitter language: {e}"))
        })?;

        let tree = parser
            .parse(source_code, None)
            .ok_or_else(|| StrataError::Internal("Tree-sitter parse failed".to_string()))?;

        let mut edges = Vec::new();

        match lang {
            LanguageKind::Rust => {
                self.extract_rust_calls(
                    tree.root_node(),
                    source_code,
                    file_path,
                    "<top-level>",
                    &mut edges,
                );
            }
            LanguageKind::TypeScript | LanguageKind::JavaScript => {
                self.extract_ts_calls(
                    tree.root_node(),
                    source_code,
                    file_path,
                    "<top-level>",
                    &mut edges,
                );
            }
            LanguageKind::Python => {
                self.extract_python_calls(
                    tree.root_node(),
                    source_code,
                    file_path,
                    "<top-level>",
                    &mut edges,
                );
            }
            LanguageKind::Unknown => {}
        }

        Ok(edges)
    }

    // ==========================================
    // Rust Call Visitor
    // ==========================================

    fn extract_rust_calls(
        &self,
        node: Node,
        source: &str,
        file_path: &str,
        current_scope: &str,
        edges: &mut Vec<CallEdge>,
    ) {
        let kind = node.kind();

        // 1. Check if entering a function/method definition to update scope
        if kind == "function_item" {
            let func_name = node
                .child_by_field_name("name")
                .and_then(|n| n.utf8_text(source.as_bytes()).ok())
                .unwrap_or(current_scope);

            let body = node.child_by_field_name("body");
            if let Some(body_node) = body {
                for i in 0..body_node.child_count() {
                    if let Some(child) = body_node.child(i) {
                        self.extract_rust_calls(child, source, file_path, func_name, edges);
                    }
                }
            }
            return;
        }

        // 2. Extract `use` imports and `mod` declarations
        if kind == "use_declaration" {
            let line = node.start_position().row as u32 + 1;
            let import_text = node.utf8_text(source.as_bytes()).unwrap_or("").trim();
            let clean_import = import_text
                .trim_start_matches("use ")
                .trim_end_matches(';')
                .trim();
            if !clean_import.is_empty() {
                edges.push(CallEdge::new(
                    file_path,
                    current_scope,
                    clean_import,
                    line,
                    CallType::Import,
                ));
            }
            return;
        }

        if kind == "mod_item" {
            let line = node.start_position().row as u32 + 1;
            if node.child_by_field_name("body").is_none() {
                if let Some(name_node) = node.child_by_field_name("name") {
                    if let Ok(mod_name) = name_node.utf8_text(source.as_bytes()) {
                        edges.push(CallEdge::new(
                            file_path,
                            current_scope,
                            format!("mod {mod_name}"),
                            line,
                            CallType::Import,
                        ));
                    }
                }
            }
        }

        // 3. Extract macro invocations (e.g. `println!`, `info!`, `vec!`)
        if kind == "macro_invocation" {
            let line = node.start_position().row as u32 + 1;
            if let Some(macro_node) = node.child_by_field_name("macro") {
                if let Ok(macro_name) = macro_node.utf8_text(source.as_bytes()) {
                    edges.push(CallEdge::new(
                        file_path,
                        current_scope,
                        format!("{macro_name}!"),
                        line,
                        CallType::MacroCall,
                    ));
                }
            }
        }

        // 4. Extract function and method call expressions
        if kind == "call_expression" {
            let line = node.start_position().row as u32 + 1;
            if let Some(func_node) = node.child_by_field_name("function") {
                let func_kind = func_node.kind();
                if func_kind == "identifier" {
                    if let Ok(callee) = func_node.utf8_text(source.as_bytes()) {
                        edges.push(CallEdge::new(
                            file_path,
                            current_scope,
                            callee,
                            line,
                            CallType::FunctionCall,
                        ));
                    }
                } else if func_kind == "scoped_identifier" {
                    if let Ok(callee) = func_node.utf8_text(source.as_bytes()) {
                        let call_type =
                            if callee.ends_with("::new") || callee.ends_with("::default") {
                                CallType::ConstructorCall
                            } else {
                                CallType::FunctionCall
                            };
                        edges.push(CallEdge::new(
                            file_path,
                            current_scope,
                            callee,
                            line,
                            call_type,
                        ));
                    }
                } else if func_kind == "field_expression" {
                    if let Some(field_node) = func_node.child_by_field_name("field") {
                        if let Ok(method_name) = field_node.utf8_text(source.as_bytes()) {
                            edges.push(CallEdge::new(
                                file_path,
                                current_scope,
                                method_name,
                                line,
                                CallType::MethodCall,
                            ));
                        }
                    }
                }
            }
        }

        // Traverse children recursively
        for i in 0..node.child_count() {
            if let Some(child) = node.child(i) {
                self.extract_rust_calls(child, source, file_path, current_scope, edges);
            }
        }
    }

    // ==========================================
    // TypeScript / JavaScript Call Visitor
    // ==========================================

    fn extract_ts_calls(
        &self,
        node: Node,
        source: &str,
        file_path: &str,
        current_scope: &str,
        edges: &mut Vec<CallEdge>,
    ) {
        let kind = node.kind();

        // 1. Function / Method declarations to update scope
        if kind == "function_declaration" || kind == "method_definition" || kind == "arrow_function"
        {
            let func_name = node
                .child_by_field_name("name")
                .and_then(|n| n.utf8_text(source.as_bytes()).ok())
                .unwrap_or(current_scope);

            let body = node.child_by_field_name("body");
            if let Some(body_node) = body {
                for i in 0..body_node.child_count() {
                    if let Some(child) = body_node.child(i) {
                        self.extract_ts_calls(child, source, file_path, func_name, edges);
                    }
                }
            }
            return;
        }

        // 2. Import statements
        if kind == "import_statement" {
            let line = node.start_position().row as u32 + 1;
            let import_source = node
                .child_by_field_name("source")
                .and_then(|n| n.utf8_text(source.as_bytes()).ok())
                .unwrap_or("")
                .trim_matches(|c| c == '\'' || c == '"');

            if !import_source.is_empty() {
                edges.push(CallEdge::new(
                    file_path,
                    current_scope,
                    import_source,
                    line,
                    CallType::Import,
                ));
            }
            return;
        }

        // 3. `new` constructor calls
        if kind == "new_expression" {
            let line = node.start_position().row as u32 + 1;
            if let Some(constructor_node) = node.child_by_field_name("constructor") {
                if let Ok(callee) = constructor_node.utf8_text(source.as_bytes()) {
                    edges.push(CallEdge::new(
                        file_path,
                        current_scope,
                        callee,
                        line,
                        CallType::ConstructorCall,
                    ));
                }
            }
        }

        // 4. Function / Method calls
        if kind == "call_expression" {
            let line = node.start_position().row as u32 + 1;
            if let Some(func_node) = node.child_by_field_name("function") {
                let func_kind = func_node.kind();
                if func_kind == "identifier" {
                    if let Ok(callee) = func_node.utf8_text(source.as_bytes()) {
                        edges.push(CallEdge::new(
                            file_path,
                            current_scope,
                            callee,
                            line,
                            CallType::FunctionCall,
                        ));
                    }
                } else if func_kind == "member_expression" {
                    if let Some(prop_node) = func_node.child_by_field_name("property") {
                        if let Ok(method_name) = prop_node.utf8_text(source.as_bytes()) {
                            edges.push(CallEdge::new(
                                file_path,
                                current_scope,
                                method_name,
                                line,
                                CallType::MethodCall,
                            ));
                        }
                    }
                }
            }
        }

        // Traverse children recursively
        for i in 0..node.child_count() {
            if let Some(child) = node.child(i) {
                self.extract_ts_calls(child, source, file_path, current_scope, edges);
            }
        }
    }

    // ==========================================
    // Python Call Visitor
    // ==========================================

    fn extract_python_calls(
        &self,
        node: Node,
        source: &str,
        file_path: &str,
        current_scope: &str,
        edges: &mut Vec<CallEdge>,
    ) {
        let kind = node.kind();

        // 1. Function / Method definition
        if kind == "function_definition" {
            let func_name = node
                .child_by_field_name("name")
                .and_then(|n| n.utf8_text(source.as_bytes()).ok())
                .unwrap_or(current_scope);

            let body = node.child_by_field_name("body");
            if let Some(body_node) = body {
                for i in 0..body_node.child_count() {
                    if let Some(child) = body_node.child(i) {
                        self.extract_python_calls(child, source, file_path, func_name, edges);
                    }
                }
            }
            return;
        }

        // 2. Import statements
        if kind == "import_statement" || kind == "import_from_statement" {
            let line = node.start_position().row as u32 + 1;
            let import_text = node.utf8_text(source.as_bytes()).unwrap_or("").trim();
            if !import_text.is_empty() {
                edges.push(CallEdge::new(
                    file_path,
                    current_scope,
                    import_text,
                    line,
                    CallType::Import,
                ));
            }
            return;
        }

        // 3. Call expressions
        if kind == "call" {
            let line = node.start_position().row as u32 + 1;
            if let Some(func_node) = node.child_by_field_name("function") {
                let func_kind = func_node.kind();
                if func_kind == "identifier" {
                    if let Ok(callee) = func_node.utf8_text(source.as_bytes()) {
                        edges.push(CallEdge::new(
                            file_path,
                            current_scope,
                            callee,
                            line,
                            CallType::FunctionCall,
                        ));
                    }
                } else if func_kind == "attribute" {
                    if let Some(attr_node) = func_node.child_by_field_name("attribute") {
                        if let Ok(method_name) = attr_node.utf8_text(source.as_bytes()) {
                            edges.push(CallEdge::new(
                                file_path,
                                current_scope,
                                method_name,
                                line,
                                CallType::MethodCall,
                            ));
                        }
                    }
                }
            }
        }

        // Traverse children recursively
        for i in 0..node.child_count() {
            if let Some(child) = node.child(i) {
                self.extract_python_calls(child, source, file_path, current_scope, edges);
            }
        }
    }

    /// Analyzes a collection of workspace files and resolves cross-file caller-callee edges.
    pub fn analyze_workspace_files(
        &self,
        files: &[(&str, &str)],
    ) -> Result<Vec<CallEdge>, StrataError> {
        let mut resolver = CrossFileResolver::new();
        resolver.resolve_workspace_files(files)
    }
}

// ==========================================
// Cross-File Dependency & Call Resolver
// ==========================================

/// Cross-file symbol and module dependency resolver.
#[derive(Debug, Default, Clone)]
pub struct CrossFileResolver {
    /// Normalized workspace file paths (using '/')
    pub workspace_files: HashSet<String>,
    /// Map from symbol name to list of files defining it
    pub symbol_to_files: HashMap<String, Vec<String>>,
    /// Map from (file_path, symbol_name) to extracted symbol
    pub file_symbols: HashMap<(String, String), ExtractedSymbol>,
    /// Map from (file_path, imported_identifier_or_specifier) to canonical target file path
    pub file_imports: HashMap<(String, String), String>,
}

impl CrossFileResolver {
    pub fn new() -> Self {
        Self::default()
    }

    /// Normalizes a path string: replaces '\\' with '/', trims leading "./"
    pub fn normalize_path(path: &str) -> String {
        let p = path.replace('\\', "/");
        p.strip_prefix("./").unwrap_or(&p).to_string()
    }

    /// Normalizes relative navigation (e.g. "src/auth" + "../utils" -> "src/utils")
    pub fn normalize_relative(dir: &str, rel: &str) -> String {
        let mut parts: Vec<&str> = dir.split('/').filter(|s| !s.is_empty()).collect();
        for seg in rel.split('/').filter(|s| !s.is_empty()) {
            if seg == "." {
                continue;
            } else if seg == ".." {
                parts.pop();
            } else {
                parts.push(seg);
            }
        }
        parts.join("/")
    }

    /// Registers extracted AST symbols for a file
    pub fn register_file_symbols(&mut self, file_path: &str, symbols: &[ExtractedSymbol]) {
        let norm_path = Self::normalize_path(file_path);
        self.workspace_files.insert(norm_path.clone());

        for sym in symbols {
            self.file_symbols
                .insert((norm_path.clone(), sym.name.clone()), sym.clone());
            self.file_symbols
                .insert((norm_path.clone(), sym.symbol_path.clone()), sym.clone());

            self.symbol_to_files
                .entry(sym.name.clone())
                .or_default()
                .push(norm_path.clone());

            if sym.symbol_path != sym.name {
                self.symbol_to_files
                    .entry(sym.symbol_path.clone())
                    .or_default()
                    .push(norm_path.clone());
            }
        }
    }

    /// Resolves a TypeScript/JavaScript relative import from `caller_file`
    pub fn resolve_ts_import(&self, caller_file: &str, import_source: &str) -> Option<String> {
        let caller_norm = Self::normalize_path(caller_file);
        let parent_dir = match caller_norm.rfind('/') {
            Some(idx) => &caller_norm[..idx],
            None => "",
        };

        let target_base = Self::normalize_relative(parent_dir, import_source);

        let extensions = [
            "",
            ".ts",
            ".tsx",
            ".js",
            ".jsx",
            ".mts",
            ".cts",
            "/index.ts",
            "/index.tsx",
            "/index.js",
            "/index.jsx",
            "/index.d.ts",
        ];

        for ext in extensions {
            let candidate = if ext.is_empty() {
                target_base.clone()
            } else {
                format!("{}{}", target_base, ext)
            };
            if self.workspace_files.contains(&candidate) {
                return Some(candidate);
            }
        }

        // Also check if candidate matches without parent_dir if import was from workspace root
        if !parent_dir.is_empty() {
            let direct_base = Self::normalize_path(
                import_source
                    .trim_start_matches("./")
                    .trim_start_matches("../"),
            );
            for ext in extensions {
                let candidate = format!("{}{}", direct_base, ext);
                if self.workspace_files.contains(&candidate) {
                    return Some(candidate);
                }
            }
        }

        None
    }

    /// Resolves a Rust module or crate path to a file in the workspace
    pub fn resolve_rust_import(&self, caller_file: &str, module_path: &str) -> Option<String> {
        let caller_norm = Self::normalize_path(caller_file);

        let crate_prefix = if let Some(idx) = caller_norm.rfind("/src/") {
            &caller_norm[..idx + 4] // includes "/src"
        } else if caller_norm.starts_with("src/") {
            "src"
        } else {
            ""
        };

        let clean_path = module_path
            .trim_start_matches("crate::")
            .trim_start_matches("self::");

        let segments: Vec<&str> = clean_path.split("::").filter(|s| !s.is_empty()).collect();
        if segments.is_empty() {
            return None;
        }

        let caller_dir = match caller_norm.rfind('/') {
            Some(idx) => &caller_norm[..idx],
            None => "",
        };

        let mut candidate_dirs = Vec::new();
        if !crate_prefix.is_empty() {
            candidate_dirs.push(crate_prefix);
        }
        if !caller_dir.is_empty() && caller_dir != crate_prefix {
            candidate_dirs.push(caller_dir);
        }
        if candidate_dirs.is_empty() {
            candidate_dirs.push("");
        }

        for base in &candidate_dirs {
            for len in (1..=segments.len()).rev() {
                let sub_path = segments[..len].join("/");
                let full_base = if base.is_empty() {
                    sub_path
                } else {
                    format!("{}/{}", base, sub_path)
                };

                let rs_file = format!("{}.rs", full_base);
                if self.workspace_files.contains(&rs_file) {
                    return Some(rs_file);
                }

                let mod_file = format!("{}/mod.rs", full_base);
                if self.workspace_files.contains(&mod_file) {
                    return Some(mod_file);
                }
            }
        }

        None
    }

    /// Resolves a Python import to a file in the workspace
    pub fn resolve_python_import(&self, caller_file: &str, module_spec: &str) -> Option<String> {
        let caller_norm = Self::normalize_path(caller_file);
        let caller_dir = match caller_norm.rfind('/') {
            Some(idx) => &caller_norm[..idx],
            None => "",
        };

        let (rel_dir, mod_clean) = if module_spec.starts_with('.') {
            let dots = module_spec.chars().take_while(|&c| c == '.').count();
            let after_dots = &module_spec[dots..];

            let mut dir = caller_dir.to_string();
            for _ in 1..dots {
                if let Some(idx) = dir.rfind('/') {
                    dir = dir[..idx].to_string();
                } else {
                    dir.clear();
                }
            }
            (dir, after_dots.replace('.', "/"))
        } else {
            (caller_dir.to_string(), module_spec.replace('.', "/"))
        };

        let target_base = if mod_clean.is_empty() {
            rel_dir.clone()
        } else if rel_dir.is_empty() {
            mod_clean
        } else {
            format!("{}/{}", rel_dir, mod_clean)
        };

        let py_file = format!("{}.py", target_base);
        if self.workspace_files.contains(&py_file) {
            return Some(py_file);
        }

        let init_file = format!("{}/__init__.py", target_base);
        if self.workspace_files.contains(&init_file) {
            return Some(init_file);
        }

        if !module_spec.starts_with('.') && !rel_dir.is_empty() {
            let root_base = module_spec.replace('.', "/");
            let root_py = format!("{}.py", root_base);
            if self.workspace_files.contains(&root_py) {
                return Some(root_py);
            }
            let root_init = format!("{}/__init__.py", root_base);
            if self.workspace_files.contains(&root_init) {
                return Some(root_init);
            }
        }

        None
    }

    /// Extracts imported symbols and specifiers from source code into `self.file_imports`
    pub fn extract_import_bindings(
        &mut self,
        file_path: &str,
        source_code: &str,
        lang: LanguageKind,
    ) {
        let caller_norm = Self::normalize_path(file_path);

        match lang {
            LanguageKind::TypeScript | LanguageKind::JavaScript => {
                for line in source_code.lines() {
                    let trimmed = line.trim();
                    if trimmed.starts_with("import ") || trimmed.starts_with("import{") {
                        if let Some(src_idx) = trimmed.find("from") {
                            let after_from = trimmed[src_idx + 4..].trim();
                            let import_src = after_from
                                .trim_matches(|c| c == '\'' || c == '"' || c == ';')
                                .trim();

                            if let Some(resolved) = self.resolve_ts_import(&caller_norm, import_src)
                            {
                                self.file_imports.insert(
                                    (caller_norm.clone(), import_src.to_string()),
                                    resolved.clone(),
                                );

                                let between = trimmed[6..src_idx].trim();
                                if between.starts_with('{') && between.contains('}') {
                                    let inner = between
                                        .trim_start_matches('{')
                                        .split('}')
                                        .next()
                                        .unwrap_or("");
                                    for part in inner.split(',') {
                                        let sym = part.split(" as ").last().unwrap_or(part).trim();
                                        if !sym.is_empty() {
                                            self.file_imports.insert(
                                                (caller_norm.clone(), sym.to_string()),
                                                resolved.clone(),
                                            );
                                        }
                                    }
                                } else if between.starts_with("* as ") {
                                    let alias = between.trim_start_matches("* as ").trim();
                                    if !alias.is_empty() {
                                        self.file_imports.insert(
                                            (caller_norm.clone(), alias.to_string()),
                                            resolved.clone(),
                                        );
                                    }
                                } else {
                                    let def_ident =
                                        between.split(',').next().unwrap_or(between).trim();
                                    if !def_ident.is_empty() {
                                        self.file_imports.insert(
                                            (caller_norm.clone(), def_ident.to_string()),
                                            resolved.clone(),
                                        );
                                    }
                                }
                            }
                        } else if trimmed.starts_with("import '")
                            || trimmed.starts_with("import \"")
                        {
                            let src = trimmed
                                .trim_start_matches("import")
                                .trim()
                                .trim_matches(|c| c == '\'' || c == '"' || c == ';');
                            if let Some(resolved) = self.resolve_ts_import(&caller_norm, src) {
                                self.file_imports
                                    .insert((caller_norm.clone(), src.to_string()), resolved);
                            }
                        }
                    } else if trimmed.contains("require(") {
                        if let Some(start) = trimmed.find("require(") {
                            let rest = &trimmed[start + 8..];
                            if let Some(end) = rest.find(')') {
                                let src = rest[..end].trim_matches(|c| c == '\'' || c == '"');
                                if let Some(resolved) = self.resolve_ts_import(&caller_norm, src) {
                                    self.file_imports.insert(
                                        (caller_norm.clone(), src.to_string()),
                                        resolved.clone(),
                                    );
                                    if trimmed.starts_with("const {")
                                        || trimmed.starts_with("let {")
                                    {
                                        if let Some(close_brace) = trimmed.find('}') {
                                            let inner = &trimmed[7..close_brace];
                                            for part in inner.split(',') {
                                                let sym = part.trim();
                                                if !sym.is_empty() {
                                                    self.file_imports.insert(
                                                        (caller_norm.clone(), sym.to_string()),
                                                        resolved.clone(),
                                                    );
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            LanguageKind::Rust => {
                for line in source_code.lines() {
                    let trimmed = line.trim();
                    if trimmed.starts_with("use ") {
                        let clean = trimmed
                            .trim_start_matches("use ")
                            .trim_end_matches(';')
                            .trim();
                        if let Some(resolved) = self.resolve_rust_import(&caller_norm, clean) {
                            self.file_imports
                                .insert((caller_norm.clone(), clean.to_string()), resolved.clone());

                            if clean.contains('{') && clean.contains('}') {
                                let brace_start = clean.find('{').unwrap();
                                let brace_end = clean.find('}').unwrap();
                                let inner = &clean[brace_start + 1..brace_end];
                                for part in inner.split(',') {
                                    let sym = part.split(" as ").last().unwrap_or(part).trim();
                                    if !sym.is_empty() {
                                        self.file_imports.insert(
                                            (caller_norm.clone(), sym.to_string()),
                                            resolved.clone(),
                                        );
                                    }
                                }
                            } else {
                                let last_seg = clean.split("::").last().unwrap_or(clean).trim();
                                let sym = last_seg.split(" as ").last().unwrap_or(last_seg).trim();
                                if !sym.is_empty() && sym != "*" {
                                    self.file_imports.insert(
                                        (caller_norm.clone(), sym.to_string()),
                                        resolved.clone(),
                                    );
                                }
                            }
                        }
                    } else if trimmed.starts_with("mod ") && trimmed.ends_with(';') {
                        let mod_name = trimmed
                            .trim_start_matches("mod ")
                            .trim_end_matches(';')
                            .trim();
                        if let Some(resolved) = self.resolve_rust_import(&caller_norm, mod_name) {
                            self.file_imports.insert(
                                (caller_norm.clone(), format!("mod {mod_name}")),
                                resolved.clone(),
                            );
                            self.file_imports
                                .insert((caller_norm.clone(), mod_name.to_string()), resolved);
                        }
                    }
                }
            }
            LanguageKind::Python => {
                for line in source_code.lines() {
                    let trimmed = line.trim();
                    if trimmed.starts_with("from ") && trimmed.contains(" import ") {
                        let parts: Vec<&str> = trimmed.split(" import ").collect();
                        if parts.len() == 2 {
                            let mod_part = parts[0].trim_start_matches("from ").trim();
                            let idents_part = parts[1].trim();

                            if let Some(resolved) =
                                self.resolve_python_import(&caller_norm, mod_part)
                            {
                                self.file_imports.insert(
                                    (caller_norm.clone(), trimmed.to_string()),
                                    resolved.clone(),
                                );
                                self.file_imports.insert(
                                    (caller_norm.clone(), mod_part.to_string()),
                                    resolved.clone(),
                                );

                                for part in idents_part.split(',') {
                                    let sym = part.split(" as ").last().unwrap_or(part).trim();
                                    if !sym.is_empty() && sym != "*" {
                                        self.file_imports.insert(
                                            (caller_norm.clone(), sym.to_string()),
                                            resolved.clone(),
                                        );
                                    }
                                }
                            }
                        }
                    } else if trimmed.starts_with("import ") {
                        let mods = trimmed.trim_start_matches("import ").trim();
                        for part in mods.split(',') {
                            let mod_name = part.split(" as ").next().unwrap_or(part).trim();
                            let alias = part.split(" as ").last().unwrap_or(part).trim();
                            if let Some(resolved) =
                                self.resolve_python_import(&caller_norm, mod_name)
                            {
                                self.file_imports.insert(
                                    (caller_norm.clone(), mod_name.to_string()),
                                    resolved.clone(),
                                );
                                self.file_imports
                                    .insert((caller_norm.clone(), alias.to_string()), resolved);
                            }
                        }
                    }
                }
            }
            LanguageKind::Unknown => {}
        }
    }

    /// Resolves and updates callee_file_hint for all edges
    pub fn resolve_edges(&self, edges: &mut [CallEdge]) {
        for edge in edges.iter_mut() {
            let caller_norm = Self::normalize_path(&edge.caller_file);

            if edge.call_type == CallType::Import {
                if let Some(target) = self
                    .file_imports
                    .get(&(caller_norm.clone(), edge.callee_symbol.clone()))
                {
                    edge.callee_file_hint = Some(target.clone());
                } else {
                    let lang = LanguageKind::from_file_path(&caller_norm);
                    let resolved = match lang {
                        LanguageKind::TypeScript | LanguageKind::JavaScript => {
                            self.resolve_ts_import(&caller_norm, &edge.callee_symbol)
                        }
                        LanguageKind::Rust => {
                            let clean = edge.callee_symbol.trim_start_matches("mod ");
                            self.resolve_rust_import(&caller_norm, clean)
                        }
                        LanguageKind::Python => {
                            let clean = if edge.callee_symbol.starts_with("from ") {
                                edge.callee_symbol
                                    .split(" import ")
                                    .next()
                                    .unwrap_or(&edge.callee_symbol)
                                    .trim_start_matches("from ")
                                    .trim()
                            } else if edge.callee_symbol.starts_with("import ") {
                                edge.callee_symbol.trim_start_matches("import ").trim()
                            } else {
                                &edge.callee_symbol
                            };
                            self.resolve_python_import(&caller_norm, clean)
                        }
                        LanguageKind::Unknown => None,
                    };
                    if let Some(res) = resolved {
                        edge.callee_file_hint = Some(res);
                    }
                }
            } else {
                // Direct symbol lookup in file_imports
                if let Some(target) = self
                    .file_imports
                    .get(&(caller_norm.clone(), edge.callee_symbol.clone()))
                {
                    edge.callee_file_hint = Some(target.clone());
                    continue;
                }

                // Scoped identifier (e.g. Type::new or module::func)
                if edge.callee_symbol.contains("::") {
                    let parts: Vec<&str> = edge.callee_symbol.split("::").collect();
                    let prefix = parts[0];
                    if let Some(target) = self
                        .file_imports
                        .get(&(caller_norm.clone(), prefix.to_string()))
                    {
                        edge.callee_file_hint = Some(target.clone());
                        continue;
                    }
                    if edge.callee_symbol.starts_with("crate::") {
                        if let Some(target) =
                            self.resolve_rust_import(&caller_norm, &edge.callee_symbol)
                        {
                            edge.callee_file_hint = Some(target);
                            continue;
                        }
                    }
                }

                // Member expression (e.g. obj.method)
                if edge.callee_symbol.contains('.') {
                    let prefix = edge.callee_symbol.split('.').next().unwrap_or("");
                    if let Some(target) = self
                        .file_imports
                        .get(&(caller_norm.clone(), prefix.to_string()))
                    {
                        edge.callee_file_hint = Some(target.clone());
                        continue;
                    }
                }

                // Check local definition in caller's file
                if self
                    .file_symbols
                    .contains_key(&(caller_norm.clone(), edge.callee_symbol.clone()))
                {
                    edge.callee_file_hint = Some(caller_norm.clone());
                    continue;
                }

                // Fallback: check unique definition across workspace
                if let Some(files) = self.symbol_to_files.get(&edge.callee_symbol) {
                    if files.len() == 1 {
                        edge.callee_file_hint = Some(files[0].clone());
                    } else if !files.is_empty() {
                        let caller_dir = caller_norm
                            .rfind('/')
                            .map(|i| &caller_norm[..i])
                            .unwrap_or("");
                        let same_dir = files.iter().find(|f| {
                            let f_dir = f.rfind('/').map(|i| &f[..i]).unwrap_or("");
                            f_dir == caller_dir
                        });
                        if let Some(f) = same_dir {
                            edge.callee_file_hint = Some(f.clone());
                        } else {
                            edge.callee_file_hint = Some(files[0].clone());
                        }
                    }
                }
            }
        }
    }

    /// End-to-end two-pass resolution for an array of (file_path, content) slices
    pub fn resolve_workspace_files(
        &mut self,
        files: &[(&str, &str)],
    ) -> Result<Vec<CallEdge>, StrataError> {
        let analyzer = CallGraphAnalyzer::new();
        let parser = AstParser::new();

        // Pass 1a: Register all files in the workspace
        for &(path, _) in files {
            self.workspace_files.insert(Self::normalize_path(path));
        }

        // Pass 1b: Parse symbols and extract import bindings
        let mut all_edges = Vec::new();
        for &(path, content) in files {
            let lang = LanguageKind::from_file_path(path);
            let norm_path = Self::normalize_path(path);

            if let Ok(symbols) = parser.parse_source(content, lang, Some(&norm_path)) {
                self.register_file_symbols(&norm_path, &symbols);
            }

            self.extract_import_bindings(&norm_path, content, lang);

            if let Ok(edges) = analyzer.analyze_source(content, lang, &norm_path) {
                all_edges.extend(edges);
            }
        }

        // Pass 2: Resolve edges cross-file
        self.resolve_edges(&mut all_edges);

        Ok(all_edges)
    }
}

// ==========================================
// Heuristic Workspace Indexer & Report
// ==========================================

/// Summary report returned by the HeuristicIndexer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct HeuristicIndexReport {
    pub files_scanned: usize,
    pub symbols_indexed: usize,
    pub call_edges_total: usize,
    pub cross_file_edges_resolved: usize,
    pub kg_edges_generated: usize,
}

/// Deep Heuristic AST and Call Graph Indexer.
pub struct HeuristicIndexer {
    analyzer: CallGraphAnalyzer,
    parser: AstParser,
}

impl Default for HeuristicIndexer {
    fn default() -> Self {
        Self::new()
    }
}

impl HeuristicIndexer {
    pub fn new() -> Self {
        Self {
            analyzer: CallGraphAnalyzer::new(),
            parser: AstParser::new(),
        }
    }

    /// Indexes an in-memory batch of workspace files: &[(&str, &str)] (file_path, content).
    /// Persists call edges into `call_edges` and tripartite Knowledge Graph edges into `memory_edges` in SQLite.
    pub fn index_files(
        &self,
        files: &[(&str, &str)],
        store: &SqliteStore,
    ) -> Result<HeuristicIndexReport, StrataError> {
        let mut resolver = CrossFileResolver::new();

        // Pass 1a: Register workspace file paths
        for &(path, _) in files {
            resolver
                .workspace_files
                .insert(CrossFileResolver::normalize_path(path));
        }

        // Pass 1b: Parse symbols and extract import bindings
        let mut all_edges = Vec::new();
        let mut total_symbols_indexed = 0;

        for &(path, content) in files {
            let lang = LanguageKind::from_file_path(path);
            let norm_path = CrossFileResolver::normalize_path(path);

            if let Ok(symbols) = self.parser.parse_source(content, lang, Some(&norm_path)) {
                total_symbols_indexed += symbols.len();
                resolver.register_file_symbols(&norm_path, &symbols);
            }

            resolver.extract_import_bindings(&norm_path, content, lang);

            if let Ok(edges) = self.analyzer.analyze_source(content, lang, &norm_path) {
                all_edges.extend(edges);
            }
        }

        // Pass 2: Resolve edges cross-file
        resolver.resolve_edges(&mut all_edges);

        let cross_file_count = all_edges
            .iter()
            .filter(|e| {
                if let Some(ref hint) = e.callee_file_hint {
                    hint != &e.caller_file
                } else {
                    false
                }
            })
            .count();

        // 1. Persist call edges idempotently into SQLite
        store.save_call_edges(&all_edges)?;

        // 2. Generate and persist Knowledge Graph memory edges
        let now = Utc::now();
        let mut kg_edges_map: HashMap<(String, String, String), MemoryEdgeRecord> = HashMap::new();

        for edge in &all_edges {
            if let Some(ref callee_file) = edge.callee_file_hint {
                if edge.call_type != CallType::Import {
                    let caller_sym_node =
                        format!("sym:{}::{}", edge.caller_file, edge.caller_symbol);
                    let callee_sym_node = format!("sym:{}::{}", callee_file, edge.callee_symbol);
                    let caller_file_node = format!("file:{}", edge.caller_file);
                    let callee_file_node = format!("file:{}", callee_file);

                    // A. sym:caller_file::caller_symbol -> sym:callee_file::callee_symbol (Calls, w=0.75)
                    kg_edges_map.insert(
                        (
                            caller_sym_node.clone(),
                            callee_sym_node.clone(),
                            EdgeKind::Calls.as_str().to_string(),
                        ),
                        MemoryEdgeRecord {
                            source_id: caller_sym_node.clone(),
                            target_id: callee_sym_node.clone(),
                            edge_kind: EdgeKind::Calls.as_str().to_string(),
                            weight: 0.75,
                            reinforcements: 0,
                            depressions: 0,
                            last_feedback_at: now,
                        },
                    );

                    // B. file:caller_file -> sym:caller_file::caller_symbol (ReferencesSymbol, w=0.85)
                    kg_edges_map.insert(
                        (
                            caller_file_node.clone(),
                            caller_sym_node.clone(),
                            EdgeKind::ReferencesSymbol.as_str().to_string(),
                        ),
                        MemoryEdgeRecord {
                            source_id: caller_file_node,
                            target_id: caller_sym_node,
                            edge_kind: EdgeKind::ReferencesSymbol.as_str().to_string(),
                            weight: 0.85,
                            reinforcements: 0,
                            depressions: 0,
                            last_feedback_at: now,
                        },
                    );

                    // C. file:callee_file -> sym:callee_file::callee_symbol (ReferencesSymbol, w=0.85)
                    kg_edges_map.insert(
                        (
                            callee_file_node.clone(),
                            callee_sym_node.clone(),
                            EdgeKind::ReferencesSymbol.as_str().to_string(),
                        ),
                        MemoryEdgeRecord {
                            source_id: callee_file_node,
                            target_id: callee_sym_node,
                            edge_kind: EdgeKind::ReferencesSymbol.as_str().to_string(),
                            weight: 0.85,
                            reinforcements: 0,
                            depressions: 0,
                            last_feedback_at: now,
                        },
                    );
                }
            }
        }

        // Also add ReferencesSymbol edges for all indexed symbols in files
        for (file_path, sym_name) in resolver.file_symbols.keys() {
            let file_node = format!("file:{}", file_path);
            let sym_node = format!("sym:{}::{}", file_path, sym_name);
            kg_edges_map
                .entry((
                    file_node.clone(),
                    sym_node.clone(),
                    EdgeKind::ReferencesSymbol.as_str().to_string(),
                ))
                .or_insert_with(|| MemoryEdgeRecord {
                    source_id: file_node,
                    target_id: sym_node,
                    edge_kind: EdgeKind::ReferencesSymbol.as_str().to_string(),
                    weight: 0.85,
                    reinforcements: 0,
                    depressions: 0,
                    last_feedback_at: now,
                });
        }

        let kg_records: Vec<MemoryEdgeRecord> = kg_edges_map.into_values().collect();
        let kg_count = kg_records.len();
        store.save_memory_edges(&kg_records)?;

        Ok(HeuristicIndexReport {
            files_scanned: files.len(),
            symbols_indexed: total_symbols_indexed,
            call_edges_total: all_edges.len(),
            cross_file_edges_resolved: cross_file_count,
            kg_edges_generated: kg_count,
        })
    }

    /// Recursively scans workspace source files on disk, indexes them, and persists to SQLite.
    pub fn index_directory(
        &self,
        workspace_root: &Path,
        store: &SqliteStore,
    ) -> Result<HeuristicIndexReport, StrataError> {
        let mut paths = Vec::new();
        let mut contents = Vec::new();
        self.collect_source_files(workspace_root, &mut paths, &mut contents)?;

        let mut relative_files = Vec::new();
        for (p, c) in paths.iter().zip(contents.iter()) {
            let rel = p
                .strip_prefix(workspace_root)
                .unwrap_or(p)
                .to_string_lossy()
                .replace('\\', "/");
            relative_files.push((rel, c.clone()));
        }

        let file_slices: Vec<(&str, &str)> = relative_files
            .iter()
            .map(|(p, c)| (p.as_str(), c.as_str()))
            .collect();

        self.index_files(&file_slices, store)
    }

    fn collect_source_files(
        &self,
        current_dir: &Path,
        out_paths: &mut Vec<PathBuf>,
        out_contents: &mut Vec<String>,
    ) -> Result<(), StrataError> {
        if !current_dir.exists() {
            return Ok(());
        }
        if current_dir.is_file() {
            if let Ok(content) = std::fs::read_to_string(current_dir) {
                out_paths.push(current_dir.to_path_buf());
                out_contents.push(content);
            }
            return Ok(());
        }
        let entries = std::fs::read_dir(current_dir).map_err(|e| {
            StrataError::Internal(format!("Failed to read dir {}: {e}", current_dir.display()))
        })?;

        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();

            if name.starts_with('.')
                || name == "target"
                || name == "node_modules"
                || name == "dist"
                || name == "build"
                || name == "venv"
                || name == ".venv"
                || name == "__pycache__"
                || name == "out"
            {
                continue;
            }

            if path.is_dir() {
                self.collect_source_files(&path, out_paths, out_contents)?;
            } else if path.is_file() {
                let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
                match ext {
                    "rs" | "ts" | "tsx" | "js" | "jsx" | "mts" | "cts" | "py" | "pyi" => {
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

// ==========================================
// TDD Tests
// ==========================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rust_call_and_import_extraction() {
        let code = r#"
use std::sync::Arc;
use crate::store::SqliteStore;

pub fn execute_plan(plan_id: &str) -> bool {
    println!("Executing plan");
    let store = SqliteStore::new();
    let is_valid = validate_token(plan_id);
    if is_valid {
        store.commit();
        true
    } else {
        false
    }
}

fn validate_token(t: &str) -> bool {
    !t.is_empty()
}
"#;

        let analyzer = CallGraphAnalyzer::new();
        let edges = analyzer
            .analyze_source(code, LanguageKind::Rust, "src/executor.rs")
            .expect("Rust AST call analysis failed");

        let graph = CallGraph::from_edges(edges);

        // 1. Verify Imports
        let imports = graph.file_imports("src/executor.rs");
        assert_eq!(imports.len(), 2);
        assert!(imports
            .iter()
            .any(|e| e.callee_symbol.contains("std::sync::Arc")));
        assert!(imports
            .iter()
            .any(|e| e.callee_symbol.contains("crate::store::SqliteStore")));

        // 2. Verify Callers of `validate_token`
        let callers = graph.callers_of("validate_token");
        assert_eq!(callers.len(), 1);
        assert_eq!(callers[0].caller_symbol, "execute_plan");
        assert_eq!(callers[0].call_type, CallType::FunctionCall);

        // 3. Verify Constructor Call `SqliteStore::new`
        let callees = graph.callees_of("src/executor.rs", "execute_plan");
        assert!(callees
            .iter()
            .any(|e| e.callee_symbol == "SqliteStore::new"
                && e.call_type == CallType::ConstructorCall));

        // 4. Verify Macro Call `println!`
        assert!(callees
            .iter()
            .any(|e| e.callee_symbol == "println!" && e.call_type == CallType::MacroCall));

        // 5. Verify Method Call `commit`
        assert!(callees
            .iter()
            .any(|e| e.callee_symbol == "commit" && e.call_type == CallType::MethodCall));
    }

    #[test]
    fn test_typescript_call_and_import_extraction() {
        let code = r#"
import { Client } from '@auth/client';
import axios from 'axios';

export async function fetchUserData(userId: string) {
    const client = new Client();
    const token = await client.getToken();
    const res = await axios.get(`/api/user/${userId}`);
    return sanitizeOutput(res.data);
}

function sanitizeOutput(data: any) {
    return JSON.stringify(data);
}
"#;

        let analyzer = CallGraphAnalyzer::new();
        let edges = analyzer
            .analyze_source(code, LanguageKind::TypeScript, "src/user.ts")
            .expect("TS call analysis failed");

        let graph = CallGraph::from_edges(edges);

        // 1. Verify imports
        let imports = graph.file_imports("src/user.ts");
        assert_eq!(imports.len(), 2);
        assert!(imports.iter().any(|e| e.callee_symbol == "@auth/client"));
        assert!(imports.iter().any(|e| e.callee_symbol == "axios"));

        // 2. Verify Constructor call `Client`
        let callees = graph.callees_of("src/user.ts", "fetchUserData");
        assert!(callees
            .iter()
            .any(|e| e.callee_symbol == "Client" && e.call_type == CallType::ConstructorCall));

        // 3. Verify Method calls `getToken` and `get`
        assert!(callees
            .iter()
            .any(|e| e.callee_symbol == "getToken" && e.call_type == CallType::MethodCall));
        assert!(callees
            .iter()
            .any(|e| e.callee_symbol == "get" && e.call_type == CallType::MethodCall));

        // 4. Verify Callers of `sanitizeOutput`
        let callers = graph.callers_of("sanitizeOutput");
        assert_eq!(callers.len(), 1);
        assert_eq!(callers[0].caller_symbol, "fetchUserData");
    }

    #[test]
    fn test_python_call_and_import_extraction() {
        let code = r#"
import os
from typing import List, Dict

def train_model(epochs: int):
    dataset = load_dataset()
    for e in range(epochs):
        step_loss = compute_loss(dataset)
        log_metrics(step_loss)

def compute_loss(data):
    return 0.42
"#;

        let analyzer = CallGraphAnalyzer::new();
        let edges = analyzer
            .analyze_source(code, LanguageKind::Python, "train.py")
            .expect("Python call analysis failed");

        let graph = CallGraph::from_edges(edges);

        // 1. Verify imports
        let imports = graph.file_imports("train.py");
        assert_eq!(imports.len(), 2);

        // 2. Verify callers of compute_loss
        let callers = graph.callers_of("compute_loss");
        assert_eq!(callers.len(), 1);
        assert_eq!(callers[0].caller_symbol, "train_model");
    }

    #[test]
    fn test_recursive_and_nested_calls() {
        let code = r#"
pub fn factorial(n: u64) -> u64 {
    if n <= 1 {
        1
    } else {
        n * factorial(n - 1)
    }
}
"#;

        let analyzer = CallGraphAnalyzer::new();
        let edges = analyzer
            .analyze_source(code, LanguageKind::Rust, "math.rs")
            .unwrap();

        let graph = CallGraph::from_edges(edges);
        let recursive = graph.detect_recursive_calls();

        assert_eq!(recursive.len(), 1);
        assert_eq!(
            recursive[0],
            ("math.rs".to_string(), "factorial".to_string())
        );
    }

    #[test]
    fn test_sqlite_call_edges_crud_and_queries() {
        use crate::store::SqliteStore;

        let store = SqliteStore::open(":memory:").expect("Failed to create in-memory store");

        let edge1 = CallEdge::new(
            "src/main.rs",
            "main",
            "init_engine",
            10,
            CallType::FunctionCall,
        );
        let edge2 = CallEdge::new(
            "src/main.rs",
            "main",
            "start_server",
            15,
            CallType::FunctionCall,
        );
        let edge3 = CallEdge::new(
            "src/api.rs",
            "handle_request",
            "start_server",
            42,
            CallType::FunctionCall,
        );
        let edge4 = CallEdge::new(
            "src/main.rs",
            "<top-level>",
            "crate::engine::init_engine",
            2,
            CallType::Import,
        );

        store
            .insert_call_edges(&[edge1.clone(), edge2.clone(), edge3.clone(), edge4.clone()])
            .expect("Failed to insert call edges");

        assert_eq!(store.get_call_edges_count().unwrap(), 4);

        // Query callers of `start_server`
        let callers = store.get_callers_of_symbol("start_server", 10).unwrap();
        assert_eq!(callers.len(), 2);
        assert!(callers
            .iter()
            .any(|e| e.caller_file == "src/main.rs" && e.caller_symbol == "main"));
        assert!(callers
            .iter()
            .any(|e| e.caller_file == "src/api.rs" && e.caller_symbol == "handle_request"));

        // Query callees from `src/main.rs:main`
        let callees = store.get_callees_of_symbol("src/main.rs", "main").unwrap();
        assert_eq!(callees.len(), 2);
        assert!(callees.iter().any(|e| e.callee_symbol == "init_engine"));
        assert!(callees.iter().any(|e| e.callee_symbol == "start_server"));

        // Query all edges in `src/main.rs`
        let file_edges = store.get_file_call_edges("src/main.rs").unwrap();
        assert_eq!(file_edges.len(), 3);

        // Clear edges for `src/main.rs`
        let deleted = store.clear_call_edges_for_file("src/main.rs").unwrap();
        assert_eq!(deleted, 3);
        assert_eq!(store.get_call_edges_count().unwrap(), 1);

        let remaining = store.get_callers_of_symbol("start_server", 10).unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].caller_file, "src/api.rs");
    }

    #[test]
    fn test_typescript_cross_file_resolution() {
        let auth_code = r#"
export function auth() {
    return "authorized";
}
"#;

        let index_code = r#"
import { auth } from './auth';

export function login() {
    return auth();
}
"#;

        let files = [("src/auth.ts", auth_code), ("src/index.ts", index_code)];

        let analyzer = CallGraphAnalyzer::new();
        let edges = analyzer
            .analyze_workspace_files(&files)
            .expect("Cross-file TS analysis failed");

        // Verify import edge resolution
        let import_edge = edges
            .iter()
            .find(|e| e.caller_file == "src/index.ts" && e.call_type == CallType::Import)
            .expect("Import edge missing");
        assert_eq!(import_edge.callee_symbol, "./auth");
        assert_eq!(import_edge.callee_file_hint.as_deref(), Some("src/auth.ts"));

        // Verify function call resolution
        let call_edge = edges
            .iter()
            .find(|e| {
                e.caller_file == "src/index.ts"
                    && e.caller_symbol == "login"
                    && e.callee_symbol == "auth"
            })
            .expect("Call edge to auth missing");
        assert_eq!(call_edge.callee_file_hint.as_deref(), Some("src/auth.ts"));
    }

    #[test]
    fn test_rust_cross_file_resolution() {
        let db_code = r#"
pub fn query(sql: &str) -> bool {
    !sql.is_empty()
}
"#;

        let main_code = r#"
use crate::db::query;

pub fn run() -> bool {
    query("SELECT 1")
}
"#;

        let files = [("src/db.rs", db_code), ("src/main.rs", main_code)];

        let analyzer = CallGraphAnalyzer::new();
        let edges = analyzer
            .analyze_workspace_files(&files)
            .expect("Cross-file Rust analysis failed");

        // Verify `use` import edge resolution
        let import_edge = edges
            .iter()
            .find(|e| e.caller_file == "src/main.rs" && e.call_type == CallType::Import)
            .expect("Import edge missing");
        assert_eq!(import_edge.callee_symbol, "crate::db::query");
        assert_eq!(import_edge.callee_file_hint.as_deref(), Some("src/db.rs"));

        // Verify function call resolution
        let call_edge = edges
            .iter()
            .find(|e| {
                e.caller_file == "src/main.rs"
                    && e.caller_symbol == "run"
                    && e.callee_symbol == "query"
            })
            .expect("Call edge to query missing");
        assert_eq!(call_edge.callee_file_hint.as_deref(), Some("src/db.rs"));
    }

    #[test]
    fn test_python_cross_file_resolution() {
        let utils_code = r#"
def helper():
    return 42
"#;

        let app_code = r#"
from .utils import helper

def run():
    return helper()
"#;

        let files = [("src/utils.py", utils_code), ("src/app.py", app_code)];

        let analyzer = CallGraphAnalyzer::new();
        let edges = analyzer
            .analyze_workspace_files(&files)
            .expect("Cross-file Python analysis failed");

        // Verify import edge resolution
        let import_edge = edges
            .iter()
            .find(|e| e.caller_file == "src/app.py" && e.call_type == CallType::Import)
            .expect("Import edge missing");
        assert_eq!(
            import_edge.callee_file_hint.as_deref(),
            Some("src/utils.py")
        );

        // Verify function call resolution
        let call_edge = edges
            .iter()
            .find(|e| {
                e.caller_file == "src/app.py"
                    && e.caller_symbol == "run"
                    && e.callee_symbol == "helper"
            })
            .expect("Call edge to helper missing");
        assert_eq!(call_edge.callee_file_hint.as_deref(), Some("src/utils.py"));
    }

    #[test]
    fn test_knowledge_graph_edges_generation_and_persistence() {
        use crate::spreading_activation::SpreadingActivationEngine;
        use crate::store::SqliteStore;

        let store = SqliteStore::open(":memory:").expect("In-memory SQLite failed");

        let auth_code = r#"
export function auth() {
    return "token";
}
"#;

        let index_code = r#"
import { auth } from './auth';

export function login() {
    auth();
}
"#;

        let files = [("src/auth.ts", auth_code), ("src/index.ts", index_code)];

        let indexer = HeuristicIndexer::new();
        let report = indexer
            .index_files(&files, &store)
            .expect("Indexing files failed");

        assert_eq!(report.files_scanned, 2);
        assert!(report.cross_file_edges_resolved >= 1);
        assert!(report.kg_edges_generated >= 3);

        // Check SQLite memory_edges table directly
        let memory_edges = store.get_all_memory_edges().unwrap();
        assert!(!memory_edges.is_empty());

        // Check Calls edge: sym:src/index.ts::login -> sym:src/auth.ts::auth with weight 0.75
        let calls_edge = memory_edges.iter().find(|e| {
            e.source_id == "sym:src/index.ts::login"
                && e.target_id == "sym:src/auth.ts::auth"
                && e.edge_kind == "Calls"
        });
        assert!(
            calls_edge.is_some(),
            "Calls edge must be present in SQLite memory_edges"
        );
        assert_eq!(calls_edge.unwrap().weight, 0.75);

        // Check file:src/index.ts -> sym:src/index.ts::login with weight 0.85
        let ref_caller = memory_edges.iter().find(|e| {
            e.source_id == "file:src/index.ts"
                && e.target_id == "sym:src/index.ts::login"
                && e.edge_kind == "ReferencesSymbol"
        });
        assert!(ref_caller.is_some());
        assert_eq!(ref_caller.unwrap().weight, 0.85);

        // Check file:src/auth.ts -> sym:src/auth.ts::auth with weight 0.85
        let ref_callee = memory_edges.iter().find(|e| {
            e.source_id == "file:src/auth.ts"
                && e.target_id == "sym:src/auth.ts::auth"
                && e.edge_kind == "ReferencesSymbol"
        });
        assert!(ref_callee.is_some());
        assert_eq!(ref_callee.unwrap().weight, 0.85);

        // Verify SpreadingActivationEngine builds graph including these edges
        let spreading_engine = SpreadingActivationEngine::with_default_config();
        let graph = spreading_engine
            .build_graph_from_store(&store, None)
            .expect("Failed to build KnowledgeGraph from store");

        assert!(graph.node_count() > 0);
        assert!(graph.edge_count() > 0);
    }

    #[test]
    fn test_heuristic_indexer_idempotency() {
        use crate::store::SqliteStore;

        let store = SqliteStore::open(":memory:").expect("In-memory SQLite failed");

        let code_a = r#"
pub fn compute() -> i32 { 10 }
"#;

        let code_b = r#"
use crate::a::compute;

pub fn execute() -> i32 {
    compute() * 2
}
"#;

        let files = [("src/a.rs", code_a), ("src/b.rs", code_b)];

        let indexer = HeuristicIndexer::new();

        // 1st indexing pass
        let report1 = indexer.index_files(&files, &store).unwrap();
        let call_count_1 = store.get_call_edges_count().unwrap();
        let memory_edge_count_1 = store.get_memory_edges_count().unwrap();

        // 2nd indexing pass (re-index same files)
        let report2 = indexer.index_files(&files, &store).unwrap();
        let call_count_2 = store.get_call_edges_count().unwrap();
        let memory_edge_count_2 = store.get_memory_edges_count().unwrap();

        assert_eq!(
            report1, report2,
            "Reports from identical index runs must match"
        );
        assert_eq!(
            call_count_1, call_count_2,
            "Call edges count must remain identical on re-index"
        );
        assert_eq!(
            memory_edge_count_1, memory_edge_count_2,
            "Memory edges count must remain identical on re-index"
        );
    }
}
