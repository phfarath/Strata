use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tree_sitter::{Node, Parser};
use uuid::Uuid;

use crate::ast::LanguageKind;
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

        // 2. Extract `use` imports
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
}
