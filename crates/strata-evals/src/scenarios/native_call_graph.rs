use std::sync::Arc;
use std::time::Instant;
use serde::{Deserialize, Serialize};

use strata_core::errors::StrataError;
use strata_memory::{
    CallGraph, CallGraphAnalyzer, LanguageKind, SqliteStore,
};

/// Evaluation scenario verifying Native Call Graph extraction performance (< 5ms) and accuracy.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallGraphEvalResult {
    pub total_files_analyzed: usize,
    pub total_edges_extracted: usize,
    pub extraction_duration_ms: u64,
    pub avg_latency_per_file_ms: f64,
    pub callers_query_duration_micros: u128,
    pub is_latency_sub_5ms: bool,
    pub recursion_detected: bool,
    pub accuracy_passed: bool,
}

pub struct NativeCallGraphEval;

impl NativeCallGraphEval {
    pub async fn run_eval() -> Result<CallGraphEvalResult, StrataError> {
        let store = Arc::new(SqliteStore::open(":memory:")?);
        let analyzer = CallGraphAnalyzer::new();

        // Multi-module benchmark source codes
        let rust_code_auth = r#"
use std::sync::Arc;
use crate::db::DatabasePool;
use crate::models::User;

pub fn authenticate_request(token: &str) -> Option<User> {
    if !validate_token_format(token) {
        return None;
    }
    let db = DatabasePool::connect();
    let user = db.query_user(token);
    user
}

fn validate_token_format(t: &str) -> bool {
    t.starts_with("Bearer ")
}
"#;

        let rust_code_db = r#"
use crate::models::User;

pub struct DatabasePool;

impl DatabasePool {
    pub fn connect() -> Self {
        DatabasePool
    }

    pub fn query_user(&self, token: &str) -> Option<User> {
        self.execute_raw_query(token);
        Some(User::default())
    }

    fn execute_raw_query(&self, q: &str) {
        println!("query: {}", q);
    }
}
"#;

        let ts_code_api = r#"
import { authenticateRequest } from './auth';
import { sendResponse } from './http';

export async function handleUserApi(req: any) {
    const user = authenticateRequest(req.headers.authorization);
    if (!user) {
        return sendResponse(401, 'Unauthorized');
    }
    return sendResponse(200, user);
}
"#;

        let py_code_worker = r#"
import json
import time

def process_queue_item(item):
    if is_valid_payload(item):
        process_queue_item(item)

def is_valid_payload(item):
    return item is not None
"#;

        let start = Instant::now();

        // 1. Analyze files
        let edges_auth = analyzer.analyze_source(rust_code_auth, LanguageKind::Rust, "src/auth.rs")?;
        let edges_db = analyzer.analyze_source(rust_code_db, LanguageKind::Rust, "src/db.rs")?;
        let edges_ts = analyzer.analyze_source(ts_code_api, LanguageKind::TypeScript, "src/api.ts")?;
        let edges_py = analyzer.analyze_source(py_code_worker, LanguageKind::Python, "worker.py")?;

        let extraction_duration = start.elapsed();
        let extraction_duration_ms = extraction_duration.as_millis() as u64;

        let mut all_edges = Vec::new();
        all_edges.extend(edges_auth);
        all_edges.extend(edges_db);
        all_edges.extend(edges_ts);
        all_edges.extend(edges_py);

        let total_files = 4;
        let total_edges = all_edges.len();
        let avg_latency = (extraction_duration_ms as f64) / (total_files as f64);
        let is_sub_5ms = avg_latency <= 5.0;

        // 2. Persist in SQLite
        store.insert_call_edges(&all_edges)?;

        // 3. Measure relational caller query speed
        let q_start = Instant::now();
        let callers = store.get_callers_of_symbol("validate_token_format", 10)?;
        let callers_query_micros = q_start.elapsed().as_micros();

        let graph = CallGraph::from_edges(all_edges);
        let recursive = graph.detect_recursive_calls();

        let recursion_detected = recursive.iter().any(|(f, sym)| f == "worker.py" && sym == "process_queue_item");
        let accuracy_passed = callers.len() == 1 && callers[0].caller_symbol == "authenticate_request" && total_edges >= 10;

        Ok(CallGraphEvalResult {
            total_files_analyzed: total_files,
            total_edges_extracted: total_edges,
            extraction_duration_ms,
            avg_latency_per_file_ms: avg_latency,
            callers_query_duration_micros: callers_query_micros,
            is_latency_sub_5ms: is_sub_5ms,
            recursion_detected,
            accuracy_passed,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_eval_native_call_graph_and_import_analyzer() {
        let eval_result = NativeCallGraphEval::run_eval().await.expect("Native call graph eval failed");
        assert!(eval_result.accuracy_passed, "Call graph extraction accuracy check failed");
        assert!(eval_result.recursion_detected, "Call graph recursion check failed");
        assert!(eval_result.is_latency_sub_5ms, "Call graph latency exceeds 5ms per file");
    }
}
