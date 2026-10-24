use anyhow::{bail, Result};
use serde_json::json;
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use strata_core::schemas::{EpisodeRecord, EpisodeStep};
use strata_memory::{SqliteStore, TrajectoryMiner};
use strata_reasoning::planning::macros::{CallbackToolInvoker, MacroExecutionEngine};

/// Scenario 30: Autonomous Procedural Skill Mining & Macro Synthesis (Phase 4.3)
///
/// Demonstrates end-to-end procedural macro lifecycle:
/// 1. Agent telemetry accumulates high-salience episodes with recurrent tool patterns.
/// 2. Trajectory miner extracts frequent n-gram tool sequences without supervision.
/// 3. Macro synthesizer generalizes concrete parameters into reusable templates.
/// 4. Procedural skill is persisted to SQLite store.
/// 5. Native 1-Step Macro Execution Engine executes the multi-step procedure
///    in a single native turn, eliminating LLM round trips and token overhead.
/// 6. Verifies fault tolerance and atomic abort on intermediate step failures.
pub async fn run_autonomous_procedural_macro_scenario() -> Result<()> {
    println!("\n--- [Scenario 30] Autonomous Procedural Skill Mining & Macro Synthesis ---");

    let store = SqliteStore::open_in_memory()?;
    let miner = TrajectoryMiner::new();

    // 1. Simulate historical episodes: 3 successful refactor runs + 1 low-salience noise run
    let mut ep1 = EpisodeRecord::new("sess-1", "claude-code-1", "Refactor auth token expiration");
    ep1.salience_score = 0.98;
    ep1.event_trace = vec![
        EpisodeStep::new(1, "Read auth.rs")
            .with_tool("read_file")
            .with_input(json!({ "path": "crates/auth/src/token.rs" })),
        EpisodeStep::new(2, "Update token ttl")
            .with_tool("edit_file")
            .with_input(
                json!({ "path": "crates/auth/src/token.rs", "content": "const TTL: u64 = 7200;" }),
            ),
        EpisodeStep::new(3, "Check compilation")
            .with_tool("cargo_check")
            .with_input(json!({ "package": "strata-auth", "flags": "--all-targets" })),
    ];

    let mut ep2 = EpisodeRecord::new("sess-2", "antigravity-1", "Refactor session store");
    ep2.salience_score = 0.96;
    ep2.event_trace = vec![
        EpisodeStep::new(1, "Read session.rs")
            .with_tool("read_file")
            .with_input(json!({ "path": "crates/session/src/store.rs" })),
        EpisodeStep::new(2, "Update max sessions")
            .with_tool("edit_file")
            .with_input(json!({ "path": "crates/session/src/store.rs", "content": "const MAX: usize = 100;" })),
        EpisodeStep::new(3, "Check compilation")
            .with_tool("cargo_check")
            .with_input(json!({ "package": "strata-session", "flags": "--all-targets" })),
    ];

    let mut ep3 = EpisodeRecord::new("sess-3", "cursor-1", "Refactor permissions matrix");
    ep3.salience_score = 0.99;
    ep3.event_trace = vec![
        EpisodeStep::new(1, "Read perms.rs")
            .with_tool("read_file")
            .with_input(json!({ "path": "crates/acl/src/perms.rs" })),
        EpisodeStep::new(2, "Add admin permission")
            .with_tool("edit_file")
            .with_input(json!({ "path": "crates/acl/src/perms.rs", "content": "enum Role { Admin, User }" })),
        EpisodeStep::new(3, "Check compilation")
            .with_tool("cargo_check")
            .with_input(json!({ "package": "strata-acl", "flags": "--all-targets" })),
    ];

    let mut ep_noise = EpisodeRecord::new("sess-4", "junior-dev", "Explore codebase random files");
    ep_noise.salience_score = 0.40;
    ep_noise.event_trace = vec![
        EpisodeStep::new(1, "Read README").with_tool("read_file"),
        EpisodeStep::new(2, "Search symbols").with_tool("symbol_search"),
    ];

    let episodes = vec![ep1, ep2, ep3, ep_noise];
    println!("  [1] Seeded 4 telemetry episodes (3 high-salience refactor sessions, 1 noise)");

    // 2. Mine frequent tool sequences
    let frequent_seqs = miner.mine_frequent_tool_sequences(&episodes, 2, 0.95);
    println!(
        "  [2] Frequent sequence mining completed: found {} recurrent patterns",
        frequent_seqs.len()
    );

    let target_seq = frequent_seqs
        .iter()
        .find(|s| s.tools == vec!["read_file", "edit_file", "cargo_check"]);

    let found_seq = match target_seq {
        Some(s) => s,
        None => bail!(
            "Expected frequent sequence ['read_file', 'edit_file', 'cargo_check'] was not mined"
        ),
    };

    println!(
        "      -> Discovered pattern: {:?} (occurrences={}, avg_salience={:.3})",
        found_seq.tools, found_seq.occurrence_count, found_seq.average_salience
    );
    if found_seq.occurrence_count < 3 {
        bail!("Occurrence count must be at least 3 for the recurring pattern");
    }

    // 3. Synthesize compiled macros
    let macros = miner.mine_and_compile_macros(&episodes, 2, 0.95);
    let compiled_macro = match macros
        .iter()
        .find(|m| m.tool_sequence == vec!["read_file", "edit_file", "cargo_check"])
    {
        Some(m) => m.clone(),
        None => bail!(
            "Compiled macro for ['read_file', 'edit_file', 'cargo_check'] was not synthesized"
        ),
    };

    println!(
        "  [3] Synthesized CompiledMacro: '{}' with {} steps, {} parameters",
        compiled_macro.name,
        compiled_macro.steps.len(),
        compiled_macro.parameters.len()
    );
    println!(
        "      Estimated savings: {} tokens, {} LLM round-trips eliminated per invocation",
        compiled_macro.estimated_tokens_saved,
        compiled_macro.steps.len().saturating_sub(1)
    );

    // 4. Persist to SqliteStore as ProceduralSkill
    let procedural_skill = compiled_macro.to_procedural_skill();
    store.insert_or_update_procedural_skill(&procedural_skill)?;

    let loaded = store
        .get_procedural_skill(&procedural_skill.id)?
        .expect("Persisted skill must exist in SqliteStore");
    println!(
        "  [4] Persisted ProceduralSkill in SQLite: id='{}', name='{}', tags={:?}",
        loaded.id, loaded.name, loaded.tags
    );
    if !loaded.tags.contains(&"compiled_macro".to_string()) {
        bail!("ProceduralSkill must contain 'compiled_macro' tag");
    }

    // 5. Native 1-Step Macro Execution: execute all 3 steps in 1 single pass
    let step_counter = Arc::new(AtomicUsize::new(0));
    let counter_clone = step_counter.clone();

    let invoker = CallbackToolInvoker::new(move |tool_name: &str, args: &serde_json::Value| {
        let _ = counter_clone.fetch_add(1, Ordering::SeqCst);
        match tool_name {
            "read_file" => {
                let path = args.get("path").and_then(|v| v.as_str()).unwrap_or("");
                Ok(json!({ "status": "ok", "path": path, "content": "// file contents..." }))
            }
            "edit_file" => {
                let path = args.get("path").and_then(|v| v.as_str()).unwrap_or("");
                let content = args.get("content").and_then(|v| v.as_str()).unwrap_or("");
                Ok(json!({ "status": "ok", "path": path, "bytes_written": content.len() }))
            }
            "cargo_check" => {
                let pkg = args.get("package").and_then(|v| v.as_str()).unwrap_or("");
                let flags = args.get("flags").and_then(|v| v.as_str()).unwrap_or("");
                Ok(json!({ "status": "ok", "package": pkg, "flags": flags, "exit_code": 0 }))
            }
            other => bail!("Unknown tool in macro: {}", other),
        }
    });

    let mut execution_params = HashMap::new();
    execution_params.insert("path".to_string(), json!("crates/strata-core/src/state.rs"));
    execution_params.insert("content".to_string(), json!("// updated state model"));
    execution_params.insert("package".to_string(), json!("strata-core"));
    execution_params.insert("flags".to_string(), json!("--all-targets"));

    let engine = MacroExecutionEngine::new();
    let report = engine
        .execute_macro_1step(&compiled_macro, &execution_params, &invoker)
        .await?;

    println!(
        "  [5] 1-Step Native Execution: success={}, total_steps={}, tokens_saved={}, duration={}ms",
        report.success, report.total_steps, report.tokens_saved, report.total_duration_ms
    );

    if !report.success {
        bail!("1-step macro execution must succeed");
    }
    if report.total_steps != 3 {
        bail!("Expected 3 executed steps, got {}", report.total_steps);
    }
    if step_counter.load(Ordering::SeqCst) != 3 {
        bail!("Tool callback must have been invoked exactly 3 times");
    }
    if report.step_results.iter().any(|s| !s.success) {
        bail!("All step results must be successful");
    }

    // 6. Fault tolerance test: verify execution halts immediately on intermediate failure
    let failing_invoker =
        CallbackToolInvoker::new(
            |tool_name: &str, _args: &serde_json::Value| match tool_name {
                "read_file" => Ok(json!({ "status": "ok" })),
                "edit_file" => bail!("Disk I/O error: permission denied writing to target file"),
                "cargo_check" => {
                    panic!("cargo_check should never be called after edit_file failed!")
                }
                other => bail!("Unexpected tool: {}", other),
            },
        );

    let failing_engine = MacroExecutionEngine::new();
    let fail_report = failing_engine
        .execute_macro_1step(&compiled_macro, &execution_params, &failing_invoker)
        .await?;

    println!(
        "  [6] Fault-Tolerance verification: halted on step {}, success={}, error={:?}",
        fail_report.step_results.len(),
        fail_report.success,
        fail_report.error
    );

    if fail_report.success {
        bail!("Execution report must indicate failure when a step fails");
    }
    if fail_report.step_results.len() != 2 {
        bail!(
            "Engine must halt at failing step 2 without executing step 3, but executed {} steps",
            fail_report.step_results.len()
        );
    }
    if fail_report.error.is_none() {
        bail!("Failure report must contain the error message from the failed step");
    }

    println!("  -> Scenario 30 passed successfully!");
    Ok(())
}
