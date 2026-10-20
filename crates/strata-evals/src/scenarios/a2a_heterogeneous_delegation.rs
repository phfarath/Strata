use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::{Duration, Instant};

use strata_core::a2a::{DelegationContract, DelegationStatus, DomainAuthority, IpcEvent};
use strata_core::errors::StrataError;
use strata_memory::{IpcClient, IpcServer, SqliteMemoryEngine};

/// Evaluation scenario measuring Heterogeneous A2A Task Delegation capabilities:
/// structured verification contracts, transactive routing to domain specialists,
/// state transitions (Pending -> Accepted -> Completed / Rejected),
/// and zero-polling synchronous waiting over local IPC (< 50ms overhead).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct A2aHeterogeneousDelegationEvalResult {
    pub delegation_contract_creation_verified: bool,
    pub transactive_specialist_resolution_verified: bool,
    pub heterogeneous_acceptance_verified: bool,
    pub subtask_execution_and_completion_verified: bool,
    pub synchronous_ipc_zero_polling_verified: bool,
    pub transactive_advisory_in_pretool_hook_verified: bool,
    pub delegation_rejection_lifecycle_verified: bool,
    pub evaluation_duration_micros: u128,
    pub is_sub_50ms: bool,
}

pub struct A2aHeterogeneousDelegationEval;

impl A2aHeterogeneousDelegationEval {
    pub async fn run_eval() -> Result<A2aHeterogeneousDelegationEvalResult, StrataError> {
        let temp_dir =
            std::env::temp_dir().join(format!("strata_delegation_eval_{}", uuid::Uuid::new_v4()));
        let _ = std::fs::create_dir_all(&temp_dir);
        let db_path = temp_dir.join("delegation_eval.db");
        let _ = std::fs::remove_file(&db_path);

        let engine = SqliteMemoryEngine::open(&db_path, None)?;
        let coord = engine.stigmergy();

        let start = Instant::now();

        // 1. Setup Transactive Domain Directory
        coord.set_domain_authority(&DomainAuthority::new(
            "crates/strata-memory",
            "agent-memory-specialist",
            0.92,
        ))?;
        coord.set_domain_authority(&DomainAuthority::new(
            "crates/strata-tools",
            "agent-tools-specialist",
            0.88,
        ))?;

        // 2. Transactive Specialist Resolution (auto-routing)
        let resolved = coord.resolve_domain_specialist("crates/strata-memory/src/store.rs")?;
        let transactive_specialist_resolution_verified = match &resolved {
            Some(auth) => {
                auth.authoritative_agent == "agent-memory-specialist"
                    && auth.confidence_score >= 0.9
            }
            None => false,
        };

        let target_agent = resolved.map(|a| a.authoritative_agent).unwrap_or_default();

        // 3. Delegation Contract Creation
        let delegation_id = "del-eval-001";
        let task_id = "task-eval-store-index";
        let contract = DelegationContract::new(
            delegation_id,
            "agent-orchestrator",
            task_id,
            "Implement high-throughput index scan in SqliteStore",
        )
        .with_delegatee(&target_agent)
        .with_domain_scope("crates/strata-memory")
        .with_context("Check table a2a_delegations indexes")
        .with_artifacts(vec!["crates/strata-memory/src/store.rs".to_string()])
        .with_verification("cargo test -p strata-memory --lib store")
        .with_timeout(30);

        coord.save_delegation(&contract)?;
        let saved = coord.get_delegation(delegation_id)?;
        let delegation_contract_creation_verified = saved.is_some()
            && saved.as_ref().unwrap().status == DelegationStatus::Pending
            && saved.as_ref().unwrap().delegatee_agent.as_deref()
                == Some("agent-memory-specialist");

        // 4. Setup Zero-Polling IPC Pipe for Real-Time Delegation Handshake
        let test_uuid = uuid::Uuid::new_v4();
        #[cfg(windows)]
        let endpoint = format!(r"\\.\pipe\strata-del-eval-{}", test_uuid);
        #[cfg(unix)]
        let endpoint = format!("/tmp/strata-del-eval-{}.sock", &test_uuid.to_string()[..8]);

        let _ipc_server = Arc::new(
            IpcServer::bind(&endpoint)
                .await
                .map_err(|e| StrataError::Io(e.to_string()))?,
        );
        let ipc_client = Arc::new(
            IpcClient::connect(&endpoint)
                .await
                .map_err(|e| StrataError::Io(e.to_string()))?,
        );
        tokio::time::sleep(Duration::from_millis(3)).await;

        // 5. Heterogeneous Acceptance Transition
        let accepted = coord.accept_delegation(delegation_id, &target_agent)?;
        let heterogeneous_acceptance_verified = accepted;

        // 6. Zero-Polling Synchronous Wait via IPC Event Stream
        // Spawn a background task representing the delegator waiting synchronously
        let client_waiter = Arc::clone(&ipc_client);
        let del_id_clone = delegation_id.to_string();
        let waiter_handle = tokio::spawn(async move {
            client_waiter
                .wait_for_event(
                    move |ev| match ev {
                        IpcEvent::DelegationCompleted { delegation_id, .. } => {
                            delegation_id == &del_id_clone
                        }
                        _ => false,
                    },
                    Duration::from_secs(5),
                )
                .await
        });

        // Delegatee executes sub-task and publishes completion event
        tokio::time::sleep(Duration::from_millis(1)).await;
        let completed_summary = "Created idx_a2a_delegations_domain and verified tests pass in 2ms";
        coord.update_delegation_status(
            delegation_id,
            DelegationStatus::Completed,
            Some(&target_agent),
            Some(completed_summary),
            None,
        )?;

        ipc_client
            .publish(IpcEvent::DelegationCompleted {
                delegation_id: delegation_id.to_string(),
                delegatee_agent: target_agent.clone(),
                success: true,
                summary: completed_summary.to_string(),
                artifacts: vec!["crates/strata-memory/src/store.rs".to_string()],
                timestamp_us: chrono::Utc::now().timestamp_micros(),
            })
            .await
            .map_err(|e| StrataError::Io(e.to_string()))?;

        let wait_result = waiter_handle
            .await
            .map_err(|e| StrataError::Io(e.to_string()))?
            .map_err(|e| StrataError::Io(e.to_string()))?;

        let synchronous_ipc_zero_polling_verified = match wait_result {
            Some(IpcEvent::DelegationCompleted {
                delegation_id: id,
                success,
                summary,
                ..
            }) => id == delegation_id && success && summary == completed_summary,
            _ => false,
        };

        let final_record = coord.get_delegation(delegation_id)?.unwrap();
        let subtask_execution_and_completion_verified = final_record.status
            == DelegationStatus::Completed
            && final_record.result_summary.as_deref() == Some(completed_summary);

        // 7. Transactive Advisory for PreTool Hook
        let hook_check = coord.resolve_domain_specialist("crates/strata-memory/src/jtms.rs")?;
        let transactive_advisory_in_pretool_hook_verified = match hook_check {
            Some(auth) => auth.authoritative_agent == "agent-memory-specialist",
            None => false,
        };

        // 8. Rejection Lifecycle Verification
        let del_reject_id = "del-eval-002";
        let reject_contract = DelegationContract::new(
            del_reject_id,
            "agent-orchestrator",
            "task-eval-out-of-scope",
            "Deploy unverified kernel module",
        )
        .with_delegatee("agent-tools-specialist");
        coord.save_delegation(&reject_contract)?;

        coord.update_delegation_status(
            del_reject_id,
            DelegationStatus::Rejected,
            Some("agent-tools-specialist"),
            None,
            Some("Capability out of scope: agent lacks root privileges"),
        )?;

        let rejected_record = coord.get_delegation(del_reject_id)?.unwrap();
        let delegation_rejection_lifecycle_verified = rejected_record.status
            == DelegationStatus::Rejected
            && rejected_record
                .error
                .as_deref()
                .unwrap_or("")
                .contains("Capability out of scope");

        let elapsed = start.elapsed();
        let elapsed_micros = elapsed.as_micros();

        let _ = std::fs::remove_dir_all(&temp_dir);

        Ok(A2aHeterogeneousDelegationEvalResult {
            delegation_contract_creation_verified,
            transactive_specialist_resolution_verified,
            heterogeneous_acceptance_verified,
            subtask_execution_and_completion_verified,
            synchronous_ipc_zero_polling_verified,
            transactive_advisory_in_pretool_hook_verified,
            delegation_rejection_lifecycle_verified,
            evaluation_duration_micros: elapsed_micros,
            is_sub_50ms: elapsed_micros < 50_000,
        })
    }
}

pub async fn run_a2a_heterogeneous_delegation_scenario() -> Result<()> {
    println!("\n--------------------------------------------------------");
    println!("Scenario: A2A Heterogeneous Orchestration & Task Delegation");
    println!("--------------------------------------------------------");

    let result = A2aHeterogeneousDelegationEval::run_eval().await?;

    println!(
        "✓ Delegation Contract Creation:             {}",
        result.delegation_contract_creation_verified
    );
    println!(
        "✓ Transactive Specialist Resolution:        {}",
        result.transactive_specialist_resolution_verified
    );
    println!(
        "✓ Heterogeneous Acceptance Lifecycle:       {}",
        result.heterogeneous_acceptance_verified
    );
    println!(
        "✓ Sub-task Execution & Completion Contract: {}",
        result.subtask_execution_and_completion_verified
    );
    println!(
        "✓ Synchronous IPC Zero-Polling Wait:        {}",
        result.synchronous_ipc_zero_polling_verified
    );
    println!(
        "✓ Transactive PreTool Hook Advisory:        {}",
        result.transactive_advisory_in_pretool_hook_verified
    );
    println!(
        "✓ Explicit Rejection Lifecycle:             {}",
        result.delegation_rejection_lifecycle_verified
    );
    println!(
        "⏱ Total Duration:                            {:.2} ms (sub-50ms: {})",
        result.evaluation_duration_micros as f64 / 1000.0,
        result.is_sub_50ms
    );

    assert!(result.delegation_contract_creation_verified);
    assert!(result.transactive_specialist_resolution_verified);
    assert!(result.heterogeneous_acceptance_verified);
    assert!(result.subtask_execution_and_completion_verified);
    assert!(result.synchronous_ipc_zero_polling_verified);
    assert!(result.transactive_advisory_in_pretool_hook_verified);
    assert!(result.delegation_rejection_lifecycle_verified);

    Ok(())
}
