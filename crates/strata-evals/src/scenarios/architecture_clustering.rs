use std::sync::Arc;
use std::time::Instant;
use serde::{Deserialize, Serialize};

use strata_core::errors::StrataError;
use strata_memory::{
    CallGraphAnalyzer, ClusteringConfig, CommunityDetector,
    LanguageKind, SqliteStore,
};

/// Evaluation scenario measuring Architecture Graph Community Extraction accuracy, modularity, and speed (< 20ms).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchitectureClusteringEvalResult {
    pub total_files_analyzed: usize,
    pub total_nodes_clustered: usize,
    pub total_edges_mapped: usize,
    pub clusters_extracted: usize,
    pub modularity: f64,
    pub clustering_duration_micros: u128,
    pub is_latency_sub_20ms: bool,
    pub accuracy_passed: bool,
    pub cache_roundtrip_passed: bool,
}

pub struct ArchitectureClusteringEval;

impl ArchitectureClusteringEval {
    pub async fn run_eval() -> Result<ArchitectureClusteringEvalResult, StrataError> {
        let analyzer = CallGraphAnalyzer::new();
        let store = Arc::new(SqliteStore::open_in_memory()?);

        // 1. Setup multi-domain source codes representing real-world architectural subsystems

        // Subsystem 1: Authentication & Security (Rust)
        let rust_auth_jwt = r#"
use std::collections::HashMap;
pub fn generate_jwt(claims: &HashMap<String, String>) -> String {
    let signature = sign_token(claims);
    format!("jwt.{}", signature)
}
fn sign_token(c: &HashMap<String, String>) -> String {
    "signed_data".to_string()
}
"#;

        let rust_auth_login = r#"
use crate::auth::jwt::generate_jwt;
use crate::db::store::query_user_by_email;

pub fn handle_login(email: &str, pass: &str) -> Option<String> {
    if verify_credentials(email, pass) {
        let user = query_user_by_email(email)?;
        Some(generate_jwt(&user))
    } else {
        None
    }
}
fn verify_credentials(e: &str, p: &str) -> bool {
    !e.is_empty() && !p.is_empty()
}
"#;

        // Subsystem 2: Database & Persistence (Rust)
        let rust_db_store = r#"
use std::collections::HashMap;
pub fn query_user_by_email(email: &str) -> Option<HashMap<String, String>> {
    let conn = open_database_connection();
    execute_sql_query(&conn, email)
}
pub fn insert_user_record(email: &str) -> bool {
    let conn = open_database_connection();
    execute_sql_query(&conn, email).is_some()
}
fn open_database_connection() -> String {
    "sqlite_conn".to_string()
}
fn execute_sql_query(c: &str, q: &str) -> Option<HashMap<String, String>> {
    Some(HashMap::new())
}
"#;

        // Subsystem 3: HTTP API & Routing (TypeScript)
        let ts_api_routes = r#"
import { handleLogin } from './auth';
import { queryUserByEmail } from './db';

export async function loginRoute(req: any) {
    const token = handleLogin(req.body.email, req.body.password);
    if (token) {
        return sendJsonResponse(200, { token });
    }
    return sendJsonResponse(401, { error: 'Unauthorized' });
}
function sendJsonResponse(code: number, data: any) {
    return { code, data };
}
"#;

        // Subsystem 4: CDC Sync Worker (Python)
        let py_sync_worker = r#"
import json
import time

def process_cdc_stream():
    events = poll_outbox_events()
    for e in events:
        replicate_delta(e)

def poll_outbox_events():
    return [{"id": 1}]

def replicate_delta(event):
    time.sleep(0.01)
"#;

        let start = Instant::now();

        // 2. Extract call and import edges across files
        let mut all_edges = Vec::new();
        all_edges.extend(analyzer.analyze_source(rust_auth_jwt, LanguageKind::Rust, "src/auth/jwt.rs")?);
        all_edges.extend(analyzer.analyze_source(rust_auth_login, LanguageKind::Rust, "src/auth/login.rs")?);
        all_edges.extend(analyzer.analyze_source(rust_db_store, LanguageKind::Rust, "src/db/store.rs")?);
        all_edges.extend(analyzer.analyze_source(ts_api_routes, LanguageKind::TypeScript, "src/api/routes.ts")?);
        all_edges.extend(analyzer.analyze_source(py_sync_worker, LanguageKind::Python, "workers/sync_worker.py")?);

        // 3. Run Community Detection & Architecture Clustering
        let config = ClusteringConfig {
            max_iterations: 25,
            min_cluster_size: 1,
            call_weight: 1.5,
            import_weight: 1.0,
        };

        let detector = CommunityDetector::new(config);
        let summary = detector.detect_from_edges(&all_edges, "eval-benchmark-workspace");

        let elapsed = start.elapsed();
        let clustering_duration_micros = elapsed.as_micros();
        let is_latency_sub_20ms = clustering_duration_micros <= 20_000;

        // 4. Validate accuracy
        let total_files = 5;
        let total_nodes = summary.total_nodes;
        let total_edges = summary.total_edges;
        let clusters_count = summary.clusters.len();
        let modularity = summary.modularity;

        // Check if clusters have positive cohesion and valid names
        let mut has_auth_cluster = false;
        let mut has_db_cluster = false;
        for c in &summary.clusters {
            if c.name.contains("Auth") || c.name.contains("Security") {
                has_auth_cluster = true;
            }
            if c.name.contains("Database") || c.name.contains("Persistence") || c.name.contains("Store") {
                has_db_cluster = true;
            }
        }

        let accuracy_passed = clusters_count >= 2
            && total_nodes >= 8
            && total_edges >= 7
            && (has_auth_cluster || has_db_cluster)
            && summary.formatted_summary.contains("High-Level Architecture Map");

        // 5. Test SQLite cache persistence and retrieval
        store.cache_architecture_summary(&summary)?;
        let cached = store.get_cached_architecture_summary("eval-benchmark-workspace")?;
        let cache_roundtrip_passed = cached.is_some() && cached.unwrap().clusters.len() == clusters_count;

        Ok(ArchitectureClusteringEvalResult {
            total_files_analyzed: total_files,
            total_nodes_clustered: total_nodes,
            total_edges_mapped: total_edges,
            clusters_extracted: clusters_count,
            modularity,
            clustering_duration_micros,
            is_latency_sub_20ms,
            accuracy_passed,
            cache_roundtrip_passed,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_eval_architecture_clustering_and_community_extraction() {
        let result = ArchitectureClusteringEval::run_eval()
            .await
            .expect("Architecture clustering eval execution failed");

        assert!(result.accuracy_passed, "Accuracy verification failed");
        assert!(result.cache_roundtrip_passed, "SQLite cache verification failed");
        assert!(
            result.is_latency_sub_20ms,
            "Latency exceeded 20ms: {} micros",
            result.clustering_duration_micros
        );
        assert!(result.clusters_extracted >= 2);
    }
}
