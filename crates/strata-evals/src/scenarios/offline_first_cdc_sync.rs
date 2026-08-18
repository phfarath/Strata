use anyhow::{bail, Result};
use strata_core::{
    schemas::{SemanticFact, SyncConfig, SyncDelta},
    state::Scope,
};
use strata_memory::{
    calculate_exponential_backoff, compute_version_hash, SqliteMemoryEngine, SyncEngine,
};
use uuid::Uuid;

/// Evaluation Scenario: Offline-First CDC Outbox Sync and Multi-Host Belief Revision
pub async fn run_offline_first_cdc_sync_scenario() -> Result<()> {
    println!("\n▶ Running Eval Scenario: Offline-First CDC Sync & Multi-Host Belief Revision");

    // 1. Setup host stores (Host A: local developer machine, Host B: remote team peer)
    let host_a_engine = SqliteMemoryEngine::open_in_memory(None)?;
    let host_a_store = host_a_engine.store_arc();

    let host_b_engine = SqliteMemoryEngine::open_in_memory(None)?;
    let host_b_store = host_b_engine.store_arc();

    // -------------------------------------------------------------------------
    // Test A: Local Delta Enqueueing in SQLite sync_outbox
    // -------------------------------------------------------------------------
    println!("  [Test A] Testing local delta enqueueing in sync_outbox...");

    let fact_id_1 = Uuid::new_v4();
    let local_fact = SemanticFact::new(
        "Strata CLI communicates with background daemon via low-latency local SQLite and stdio IPC",
        "architecture",
        Scope::Project("strata".to_string()),
    )
    .with_id(fact_id_1)
    .with_importance(0.9)
    .with_confidence(1.0);

    let fact_payload = serde_json::to_value(&local_fact)?;
    let version_hash = compute_version_hash(&fact_payload);

    let delta_1 = SyncDelta::new(
        "ws-strata-primary",
        1,
        "semantic_fact",
        fact_payload.clone(),
        version_hash.clone(),
    );

    host_a_store.insert_or_update_semantic_fact(&local_fact)?;
    host_a_store.enqueue_delta(&delta_1)?;

    let pending_deltas = host_a_store.get_pending_deltas("ws-strata-primary", 10)?;
    if pending_deltas.is_empty() {
        bail!("Expected at least 1 pending delta in sync_outbox, found 0");
    }
    assert_eq!(pending_deltas[0].id, delta_1.id);
    assert_eq!(pending_deltas[0].version_hash, version_hash);
    println!("    ✓ Enqueued delta [{}] with version hash {}", delta_1.id, version_hash);

    let (pending_count, max_seq) = host_a_store.get_sync_status("ws-strata-primary")?;
    assert_eq!(pending_count, 1);
    assert_eq!(max_seq, 1);
    println!("    ✓ Sync status confirmed: pending={}, max_seq={}", pending_count, max_seq);

    // -------------------------------------------------------------------------
    // Test B: Offline Retry Handling with Exponential Backoff
    // -------------------------------------------------------------------------
    println!("  [Test B] Testing offline retry handling and exponential backoff...");

    // Test backoff calculation formula
    let b0 = calculate_exponential_backoff(0, 500, 30000);
    let b1 = calculate_exponential_backoff(1, 500, 30000);
    let b2 = calculate_exponential_backoff(2, 500, 30000);
    let b3 = calculate_exponential_backoff(3, 500, 30000);

    assert_eq!(b0.as_millis(), 500);
    assert_eq!(b1.as_millis(), 1000);
    assert_eq!(b2.as_millis(), 2000);
    assert_eq!(b3.as_millis(), 4000);
    println!("    ✓ Mathematical backoff sequence verified: 500ms -> 1000ms -> 2000ms -> 4000ms");

    // Simulate transient network failure on Host A delta transmission
    let delta_ids = vec![delta_1.id];
    host_a_store.record_delta_failure(&delta_ids, "Simulated 503 Service Unavailable / Offline")?;

    // Verify delta is currently delayed / backing off
    let pending_after_failure = host_a_store.get_pending_deltas("ws-strata-primary", 10)?;
    if !pending_after_failure.is_empty() {
        bail!("Delta should be deferred by next_retry_ts backoff window");
    }
    println!("    ✓ Delta correctly deferred according to exponential retry backoff schedule");

    // Simulate successful sync delivery once connectivity is restored
    host_a_store.mark_deltas_synced(&delta_ids)?;
    let (pending_synced, _) = host_a_store.get_sync_status("ws-strata-primary")?;
    assert_eq!(pending_synced, 0);
    println!("    ✓ Delta marked synced successfully after recovery");

    // -------------------------------------------------------------------------
    // Test C: Remote Delta Pull and JTMS Multi-Host Belief Conflict Resolution
    // -------------------------------------------------------------------------
    println!("  [Test C] Testing remote delta pull and JTMS multi-host belief conflict resolution...");

    // Setup initial belief on Host B (e.g. older architectural fact)
    let initial_fact_b = SemanticFact::new(
        "Strata uses a centralized daemon requiring persistent background TCP listening on port 9090",
        "architecture",
        Scope::Project("strata".to_string()),
    )
    .with_id(fact_id_1)
    .with_importance(0.8)
    .with_confidence(0.7);

    host_b_store.insert_or_update_semantic_fact(&initial_fact_b)?;

    // Create SyncEngine for Host B
    let config_b = SyncConfig::new("ws-strata-primary");
    let sync_engine_b = SyncEngine::new(host_b_store.clone(), config_b);

    // Host B pulls incoming delta from Host A containing updated belief
    let incoming_remote_deltas = vec![SyncDelta::new(
        "ws-strata-primary",
        2,
        "semantic_fact",
        serde_json::to_value(&local_fact)?,
        version_hash.clone(),
    )];

    let applied = sync_engine_b.pull_deltas(incoming_remote_deltas).await?;
    if applied != 1 {
        bail!("Expected 1 delta applied during pull, got {applied}");
    }

    // Verify JTMS updated Host B belief state
    let updated_fact_b = host_b_store
        .get_semantic_fact(&fact_id_1)?
        .expect("Fact should exist on Host B");

    if updated_fact_b.statement != local_fact.statement {
        bail!(
            "Conflict resolution failed on Host B! Expected '{}', got '{}'",
            local_fact.statement,
            updated_fact_b.statement
        );
    }
    println!("    ✓ JTMS resolved multi-host belief: updated statement on Host B to '{}'", updated_fact_b.statement);

    println!("  ✓ Offline-First CDC Sync evaluation scenario PASSED (3/3 tests).\n");
    Ok(())
}
