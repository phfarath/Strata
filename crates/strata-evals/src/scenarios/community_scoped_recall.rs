use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Instant;

use strata_core::errors::StrataError;
use strata_core::state::{MemoryRecord, MemoryType, Scope};
use strata_memory::{
    CallGraphAnalyzer, ClusteringConfig, CommunityDetector, CommunityMemoryRouter, HybridRanker,
    LanguageKind, SqliteStore,
};

/// Evaluation results for Community-Scoped Memory Recall Benchmark.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommunityScopedRecallEvalResult {
    pub candidate_reduction_pct: f64,
    pub candidate_reduction_passed: bool, // >= 60%
    pub determinism_runs: usize,
    pub determinism_passed: bool,          // 100% match across 5 runs
    pub cold_start_fallback_passed: bool,  // Transparent fallback when < k
    pub assignment_migration_passed: bool, // Active memories preserved across re-clustering
    pub top_k_accuracy_passed: bool,       // Top-k labeled queries retrieve correct domain memories
    pub eval_duration_micros: u128,
}

pub type CommunityScopedRecallResult = CommunityScopedRecallEvalResult;

pub struct CommunityScopedRecallEval;

impl CommunityScopedRecallEval {
    pub async fn run_eval() -> Result<CommunityScopedRecallEvalResult, StrataError> {
        let start = Instant::now();

        // 1. Verify Determinism across runs with fixed seed
        let analyzer = CallGraphAnalyzer::new();
        let code_subsystems = r#"
// Auth
pub fn auth_login() { auth_jwt(); }
pub fn auth_jwt() { auth_sign(); }
pub fn auth_sign() { auth_login(); }

// Database
pub fn db_query() { db_pool(); }
pub fn db_pool() { db_exec(); }
pub fn db_exec() { db_query(); }

// API Routing
pub fn api_route() { api_handler(); }
pub fn api_handler() { api_response(); }
pub fn api_response() { api_route(); }

// Sync
pub fn sync_worker() { sync_poll(); }
pub fn sync_poll() { sync_apply(); }
pub fn sync_apply() { sync_worker(); }
"#;
        let edges =
            analyzer.analyze_source(code_subsystems, LanguageKind::Rust, "src/subsystems.rs")?;

        let config = ClusteringConfig {
            seed: 42,
            ..Default::default()
        };
        let detector = CommunityDetector::new(config);

        let baseline_summary = detector.detect_from_edges(&edges, "eval-workspace");
        let mut determinism_passed = true;
        for _ in 0..4 {
            let next_summary = detector.detect_from_edges(&edges, "eval-workspace");
            if next_summary.clusters.len() != baseline_summary.clusters.len()
                || next_summary.modularity != baseline_summary.modularity
            {
                determinism_passed = false;
                break;
            }
            for (c1, c2) in baseline_summary
                .clusters
                .iter()
                .zip(next_summary.clusters.iter())
            {
                if c1.id != c2.id || c1.members.len() != c2.members.len() {
                    determinism_passed = false;
                    break;
                }
            }
        }

        // 2. Setup Database with 4 distinct community subsystems (10 memories each = 40 memories)
        let store = Arc::new(SqliteStore::open_in_memory()?);
        let ranker = HybridRanker::with_default_config();

        let cluster_auth = "cluster-1-auth";
        let cluster_db = "cluster-2-db";
        let cluster_api = "cluster-3-api";
        let cluster_sync = "cluster-4-sync";

        for i in 0..10 {
            store.insert_or_update_memory(&MemoryRecord::new(
                MemoryType::Semantic,
                format!(
                    "Authentication invariant {i}: RSA 256 JWT bearer signing and secret rotation"
                ),
                Scope::Community(cluster_auth.to_string()),
            ))?;
            store.insert_or_update_memory(&MemoryRecord::new(
                MemoryType::Semantic,
                format!("Database invariant {i}: PostgreSQL transaction isolation level Serializable and write-ahead log"),
                Scope::Community(cluster_db.to_string()),
            ))?;
            store.insert_or_update_memory(&MemoryRecord::new(
                MemoryType::Semantic,
                format!("HTTP API invariant {i}: Rate limiter token bucket with 100 req/min threshold and JSON payload"),
                Scope::Community(cluster_api.to_string()),
            ))?;
            store.insert_or_update_memory(&MemoryRecord::new(
                MemoryType::Semantic,
                format!("CDC Sync invariant {i}: Outbox pattern with idempotency key and exponential backoff retry"),
                Scope::Community(cluster_sync.to_string()),
            ))?;
        }

        let total_memories = store.count_all_memories()?;
        assert_eq!(total_memories, 40);

        // 3. Test Community Scoped Recall: Candidate Pool Reduction >= 60% and Top-k Accuracy
        let router = CommunityMemoryRouter::new(5, true);
        let recall_result = router
            .retrieve_scoped(
                &ranker,
                &store,
                None,
                "RSA 256 JWT bearer signing",
                cluster_auth,
                Some(&baseline_summary),
                None,
                5,
            )
            .await?;

        let candidate_reduction_pct = recall_result.candidate_reduction_pct;
        let candidate_reduction_passed = candidate_reduction_pct >= 60.0;

        let mut top_k_accuracy_passed = !recall_result.memories.is_empty();
        for mem in &recall_result.memories {
            if !mem.content.contains("Authentication")
                && mem.scope != Scope::Community(cluster_auth.to_string())
            {
                top_k_accuracy_passed = false;
            }
        }

        // 4. Test Transparent Cold-Start Fallback for clusters with < k memories
        let cluster_cold = "cluster-cold-start";
        for i in 0..2 {
            store.insert_or_update_memory(&MemoryRecord::new(
                MemoryType::Semantic,
                format!("Cold start snippet {i}: early prototype idea"),
                Scope::Community(cluster_cold.to_string()),
            ))?;
        }
        let cold_recall = router
            .retrieve_scoped(
                &ranker,
                &store,
                None,
                "transaction isolation Serializable",
                cluster_cold,
                Some(&baseline_summary),
                None,
                5,
            )
            .await?;

        let cold_start_fallback_passed =
            cold_recall.fallback_to_global && !cold_recall.memories.is_empty();

        // 5. Test Re-Clustering Assignment Migration
        // Create an active memory in a cluster that will undergo re-clustering
        let mem_to_migrate = MemoryRecord::new(
            MemoryType::Semantic,
            "Critical session validation token rule",
            Scope::Community(cluster_auth.to_string()),
        );
        store.insert_or_update_memory(&mem_to_migrate)?;

        // Simulate code modification adding new auth edges, yielding new summary
        let modified_code = format!("{}\npub fn auth_oauth() {{ auth_jwt(); }}", code_subsystems);
        let mod_edges =
            analyzer.analyze_source(&modified_code, LanguageKind::Rust, "src/subsystems.rs")?;
        let next_summary = detector.detect_from_edges(&mod_edges, "eval-workspace");

        let migration_plan = detector.compute_migration_plan(&baseline_summary, &next_summary);
        let migration_passed = !migration_plan.mapping.is_empty();
        let _migrated = detector.migrate_active_memories(&store, &migration_plan)?;

        let retrieved_mem = store.get_memory(&mem_to_migrate.id)?;
        let assignment_migration_passed = migration_passed
            && retrieved_mem.is_some()
            && retrieved_mem.unwrap().scope.as_community().is_some();

        let elapsed = start.elapsed();

        Ok(CommunityScopedRecallEvalResult {
            candidate_reduction_pct,
            candidate_reduction_passed,
            determinism_runs: 5,
            determinism_passed,
            cold_start_fallback_passed,
            assignment_migration_passed,
            top_k_accuracy_passed,
            eval_duration_micros: elapsed.as_micros(),
        })
    }
}

pub async fn run_community_scoped_recall_scenario() -> anyhow::Result<()> {
    let res = CommunityScopedRecallEval::run_eval().await?;
    println!(
        "  -> Candidate Pool Reduction: {:.1}% (passed: {})",
        res.candidate_reduction_pct, res.candidate_reduction_passed
    );
    println!(
        "  -> Determinism across {} runs: {}",
        res.determinism_runs, res.determinism_passed
    );
    println!(
        "  -> Cold-start Transparent Fallback: {}",
        res.cold_start_fallback_passed
    );
    println!(
        "  -> Re-clustering Assignment Migration: {}",
        res.assignment_migration_passed
    );
    println!(
        "  -> Top-k Domain Recall Accuracy: {}",
        res.top_k_accuracy_passed
    );
    println!("  -> Execution time: {} µs", res.eval_duration_micros);

    if !res.candidate_reduction_passed {
        anyhow::bail!(
            "Candidate pool reduction failed: {:.1}% < 60.0%",
            res.candidate_reduction_pct
        );
    }
    if !res.determinism_passed {
        anyhow::bail!("Determinism check failed across runs with fixed seed");
    }
    if !res.cold_start_fallback_passed {
        anyhow::bail!("Cold start fallback verification failed");
    }
    if !res.assignment_migration_passed {
        anyhow::bail!("Assignment migration verification failed");
    }
    if !res.top_k_accuracy_passed {
        anyhow::bail!("Top-k accuracy verification failed");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_eval_community_scoped_recall() {
        let result = CommunityScopedRecallEval::run_eval()
            .await
            .expect("Community scoped recall eval failed");

        assert!(
            result.candidate_reduction_passed,
            "Candidate reduction >= 60% failed"
        );
        assert!(result.determinism_passed, "Determinism failed");
        assert!(
            result.cold_start_fallback_passed,
            "Cold start fallback failed"
        );
        assert!(
            result.assignment_migration_passed,
            "Assignment migration failed"
        );
        assert!(result.top_k_accuracy_passed, "Top-k accuracy failed");
    }
}
