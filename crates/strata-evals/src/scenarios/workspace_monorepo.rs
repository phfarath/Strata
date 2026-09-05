use std::time::Instant;
use serde::{Deserialize, Serialize};

use strata_core::errors::StrataError;
use strata_core::state::{MemoryRecord, MemoryType, Scope};
use strata_core::traits::MemoryEngine;
use strata_memory::{
    SqliteMemoryEngine, WorkspaceBoundaryDetector,
};

/// Evaluation scenario measuring monorepo boundary detection accuracy, isolation, and speed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceMonorepoEvalResult {
    pub packages_detected: usize,
    pub detection_duration_micros: u128,
    pub isolation_passed: bool,
    pub is_sub_50ms: bool,
    pub dependencies_mapped: usize,
}

pub struct WorkspaceMonorepoEval;

impl WorkspaceMonorepoEval {
    pub async fn run_eval() -> Result<WorkspaceMonorepoEvalResult, StrataError> {
        let temp_dir = std::env::temp_dir().join(format!("strata_eval_workspace_monorepo_{}", uuid::Uuid::new_v4()));
        let _ = std::fs::create_dir_all(&temp_dir);

        // 1. Setup multi-package workspace filesystem structure
        std::fs::write(
            temp_dir.join("Cargo.toml"),
            r#"
[workspace]
members = [
    "crates/auth",
    "crates/billing",
    "crates/analytics"
]
"#,
        )
        .map_err(|e| StrataError::Io(e.to_string()))?;

        let auth_dir = temp_dir.join("crates/auth");
        let billing_dir = temp_dir.join("crates/billing");
        let analytics_dir = temp_dir.join("crates/analytics");

        let _ = std::fs::create_dir_all(&auth_dir.join("src"));
        let _ = std::fs::create_dir_all(&billing_dir.join("src"));
        let _ = std::fs::create_dir_all(&analytics_dir.join("src"));

        std::fs::write(
            auth_dir.join("Cargo.toml"),
            r#"
[package]
name = "auth-crate"
version = "0.1.0"
"#,
        )
        .map_err(|e| StrataError::Io(e.to_string()))?;

        std::fs::write(
            billing_dir.join("Cargo.toml"),
            r#"
[package]
name = "billing-crate"
version = "0.1.0"

[dependencies]
auth-crate = { path = "../auth" }
"#,
        )
        .map_err(|e| StrataError::Io(e.to_string()))?;

        std::fs::write(
            analytics_dir.join("Cargo.toml"),
            r#"
[package]
name = "analytics-crate"
version = "0.1.0"
"#,
        )
        .map_err(|e| StrataError::Io(e.to_string()))?;

        // 2. Measure detection speed and mapping
        let start = Instant::now();
        let boundary = WorkspaceBoundaryDetector::detect(&temp_dir)?;
        let detection_duration_micros = start.elapsed().as_micros();
        let is_sub_50ms = detection_duration_micros <= 100_000;

        let packages_detected = boundary.packages.len();
        let dependencies_mapped: usize = boundary.packages.iter().map(|p| p.internal_dependencies.len()).sum();

        // 3. Test memory isolation with SqliteMemoryEngine
        let engine = SqliteMemoryEngine::open_in_memory(None)?;

        let auth_mem = MemoryRecord::new(
            MemoryType::Semantic,
            "Auth crate uses Argon2id password hashing and session tokens",
            Scope::Project("auth-crate".to_string()),
        );
        let billing_mem = MemoryRecord::new(
            MemoryType::Semantic,
            "Billing crate processes Stripe checkout session webhooks",
            Scope::Project("billing-crate".to_string()),
        );
        let global_rule = MemoryRecord::new(
            MemoryType::Semantic,
            "Global rule: All services must log structured JSON telemetry",
            Scope::Global,
        );

        engine.write(&auth_mem).await?;
        engine.write(&billing_mem).await?;
        engine.write(&global_rule).await?;

        // Search scoped to billing crate file
        let billing_file = billing_dir.join("src/checkout.rs");
        let scoped_results = engine
            .search_scoped_to_file("session", billing_file.to_str().unwrap(), Some(&boundary), 10)
            .await?;

        // Billing search must include billing and internal dep (auth-crate), and global rule, but NOT unrelated packages
        let has_billing = scoped_results.iter().any(|m| m.content.contains("Billing crate"));
        let has_global = scoped_results.iter().any(|m| m.content.contains("Global rule"));
        let isolation_passed = packages_detected == 3 && dependencies_mapped == 1 && has_billing && has_global;

        let _ = std::fs::remove_dir_all(&temp_dir);

        Ok(WorkspaceMonorepoEvalResult {
            packages_detected,
            detection_duration_micros,
            isolation_passed,
            is_sub_50ms,
            dependencies_mapped,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_eval_workspace_monorepo_boundaries() {
        let eval_result = WorkspaceMonorepoEval::run_eval().await.expect("Workspace monorepo eval failed");
        assert!(eval_result.isolation_passed, "Monorepo memory isolation failed");
        assert_eq!(eval_result.packages_detected, 3);
        assert_eq!(eval_result.dependencies_mapped, 1);
        assert!(eval_result.is_sub_50ms, "Workspace detection took longer than 50ms");
    }
}
