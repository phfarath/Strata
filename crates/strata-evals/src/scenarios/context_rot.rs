use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use strata_core::state::{MemoryRecord, MemoryType, Scope};
use strata_core::traits::MemoryEngine;
use strata_memory::{
    ContextRotDetector, InjectionOutcome, MemoryInjectionRecord, SqliteMemoryEngine,
};

/// Evaluation report summarizing Context Rot & Context Poisoning runtime detection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextRotEvalReport {
    pub poisoned_memory_detected: bool,
    pub suppression_verified: bool,
    pub rejection_rate: f32,
    pub total_injections: usize,
    pub active_memory_retained: bool,
}

/// Scenario: Context Rot & Context Poisoning Runtime Detector
/// Simulates injecting valid and poisoned advice into execution contexts,
/// measuring command outcomes, detecting poisoning when failure rate > 60%,
/// and verifying automatic penalty decay and context suppression.
pub async fn run_context_rot_scenario() -> Result<ContextRotEvalReport> {
    println!("\n▶ Running Eval Scenario: Context Rot & Context Poisoning Runtime Detection");

    let session_id = "eval-sess-rot-01";
    let engine = SqliteMemoryEngine::open_in_memory(None)?;
    let store = engine.store();

    // 1. Ingest Memory 1: Valid advice
    let valid_mem = MemoryRecord::new(
        MemoryType::Semantic,
        "Run cargo check to verify types and fast syntax check before compilation.",
        Scope::Session(session_id.to_string()),
    )
    .with_summary("cargo check verification")
    .with_importance(0.85)
    .with_confidence(0.95);
    engine.write(&valid_mem).await?;

    // 2. Ingest Memory 2: Poisoned advice (e.g. outdated command that fails)
    let poisoned_mem = MemoryRecord::new(
        MemoryType::Semantic,
        "Always execute deprecated python2 script to generate legacy mock stubs.",
        Scope::Session(session_id.to_string()),
    )
    .with_summary("python2 legacy stubs")
    .with_importance(0.85)
    .with_confidence(0.95);
    engine.write(&poisoned_mem).await?;

    // 3. Simulate post-generation injection outcomes
    // Memory 1 gets 5 successful injections -> rejection rate 0.0%
    for i in 0..5 {
        let rec = MemoryInjectionRecord::new(valid_mem.id, session_id, InjectionOutcome::Success)
            .with_task_id(format!("task-valid-{i}"))
            .with_command("cargo check");
        store.record_injection_outcome(&rec)?;
    }

    // Memory 2 gets 4 failures and 1 success out of 5 injections -> rejection rate 80% (> 60%)
    for i in 0..4 {
        let rec =
            MemoryInjectionRecord::new(poisoned_mem.id, session_id, InjectionOutcome::Failure)
                .with_task_id(format!("task-poison-{i}"))
                .with_command("python2 scripts/generate_stubs.py");
        store.record_injection_outcome(&rec)?;
    }
    let rec_succ =
        MemoryInjectionRecord::new(poisoned_mem.id, session_id, InjectionOutcome::Success)
            .with_task_id("task-poison-4")
            .with_command("python2 scripts/generate_stubs.py");
    store.record_injection_outcome(&rec_succ)?;

    // 4. Run ContextRotDetector
    let detector = ContextRotDetector::with_default_config();

    // Evaluate efficacy of each memory
    let valid_metric = detector.evaluate_memory_efficacy(store, &valid_mem.id)?;
    if valid_metric.is_poisoned {
        bail!("Valid memory was falsely flagged as poisoned!");
    }
    if valid_metric.rejection_rate != 0.0 {
        bail!(
            "Valid memory rejection rate should be 0.0, got {}",
            valid_metric.rejection_rate
        );
    }

    let poison_metric = detector.evaluate_memory_efficacy(store, &poisoned_mem.id)?;
    if !poison_metric.is_poisoned {
        bail!("Poisoned memory was NOT detected as poisoned!");
    }
    if poison_metric.rejection_rate <= 0.60 {
        bail!(
            "Poisoned memory rejection rate should be > 0.60, got {}",
            poison_metric.rejection_rate
        );
    }

    // Detect poisoned memories
    let poisoned_list = detector.detect_poisoned_memories(store)?;
    if poisoned_list.len() != 1 || poisoned_list[0].memory_id != poisoned_mem.id {
        bail!(
            "Expected exactly 1 poisoned memory (poisoned_mem), found: {:?}",
            poisoned_list
        );
    }

    // 5. Apply automatic penalty decay and quarantine
    let decayed_count = detector.apply_penalty_decay(store)?;
    if decayed_count != 1 {
        bail!(
            "Expected 1 memory to be decayed and quarantined, got {}",
            decayed_count
        );
    }

    let is_poisoned_quarantined = store.is_memory_quarantined(&poisoned_mem.id)?;
    let is_valid_quarantined = store.is_memory_quarantined(&valid_mem.id)?;
    if !is_poisoned_quarantined {
        bail!("Poisoned memory must be marked as quarantined in SQLite store!");
    }
    if is_valid_quarantined {
        bail!("Valid memory must NOT be quarantined!");
    }

    // 6. Verify poisoned memory is suppressed from subsequent search and digest
    let search_results = engine.search("python2 script", None, 10).await?;
    let poison_in_search = search_results.iter().any(|m| m.id == poisoned_mem.id);
    if poison_in_search {
        bail!("Poisoned memory was still returned in search after quarantine!");
    }

    let search_valid = engine.search("cargo check", None, 10).await?;
    let valid_in_search = search_valid.iter().any(|m| m.id == valid_mem.id);
    if !valid_in_search {
        bail!("Valid memory should be returned in search!");
    }

    let digest = engine.digest(session_id, None).await?;
    let poison_in_digest = digest.key_pointers.iter().any(|h| h.id == poisoned_mem.id);
    let valid_in_digest = digest.key_pointers.iter().any(|h| h.id == valid_mem.id);
    if poison_in_digest {
        bail!("Poisoned memory was still included in digest key_pointers!");
    }
    if !valid_in_digest {
        bail!("Valid memory should be included in digest key_pointers!");
    }

    // 7. Test in-memory filtering via filter_suppressed_memories
    let mut candidate_records = vec![valid_mem.clone(), poisoned_mem.clone()];
    detector.filter_suppressed_memories(store, &mut candidate_records)?;
    if candidate_records.len() != 1 || candidate_records[0].id != valid_mem.id {
        bail!("filter_suppressed_memories failed to filter out poisoned memory!");
    }

    println!(
        "  ✔ Poisoned memory detected: rejection rate {:.1}% > 60%",
        poison_metric.rejection_rate * 100.0
    );
    println!("  ✔ Decay penalty applied: importance and confidence penalized, memory quarantined");
    println!("  ✔ Context rot prevented: poisoned memory suppressed from recall and digest");

    Ok(ContextRotEvalReport {
        poisoned_memory_detected: true,
        suppression_verified: true,
        rejection_rate: poison_metric.rejection_rate,
        total_injections: poison_metric.total_injections,
        active_memory_retained: true,
    })
}
