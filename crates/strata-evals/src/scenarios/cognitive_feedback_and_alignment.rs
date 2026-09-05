use anyhow::{bail, Result};
use chrono::Utc;
use std::fs;
use std::sync::Arc;
use strata_core::events::{
    ErrorObserved, Event, EventPayload, SessionEnded, SessionStarted, TaskCompleted, TaskStarted,
    ToolInvoked, ToolResultReceived,
};
use strata_core::schemas::{MemoryFeedback, ParameterDef, ProceduralSkill, ProceduralStep};
use strata_core::state::{FailurePattern, FailureSeverity, MemoryRecord, MemoryType, Scope};
use strata_memory::{
    DpoPair, ExportFormat, KtoSample, MultiHostCompiler, PreferenceMiner, SftSample, SqliteStore,
};
use uuid::Uuid;

/// Scenario 8: Cognitive Feedback, Preference Mining & Multi-Host Context Alignment
/// Evaluates:
/// 1. Recording implicit signals (ToolLoop / CommandFix) and explicit memory feedback in SQLite.
/// 2. Mining DPO preference pairs (failure trajectory vs resolution) and validating format.
/// 3. Mining KTO binary alignment samples and SFT procedural skills.
/// 4. Multi-host context compilation (.cursor/rules/strata.mdc, CLAUDE.md, AGENTS.md, .gemini/GEMINI.md) respecting token budget.
pub async fn run_cognitive_feedback_and_alignment_scenario() -> Result<()> {
    println!(
        "\n▶ Running Eval Scenario: Cognitive Feedback, Preference Mining & Multi-Host Alignment"
    );

    // 1. Setup in-memory SQLite store
    let store = SqliteStore::open_in_memory()?;
    let store_arc = Arc::new(store);

    let session_id = "sess-alignment-feedback-88";
    let agent_id = "agent-cognitive-eval";
    let start_time = Utc::now();

    // ========================================================================
    // Part A: Record Implicit Signals (ToolLoop / CommandFix) & Explicit Feedback
    // ========================================================================
    println!("  [Step A] Recording implicit event signals and explicit feedback...");

    // Event Stream: Session Started -> Task Started -> Failed Command -> Error -> Fixed Command -> Task Success
    let mut events = Vec::new();

    events.push(Event::new(
        session_id,
        agent_id,
        EventPayload::SessionStarted(SessionStarted {
            session_id: session_id.to_string(),
            agent_id: agent_id.to_string(),
            organization_id: None,
            environment: serde_json::json!({ "os": "windows" }),
            timestamp: start_time,
        }),
    ));

    events.push(Event::new(
        session_id,
        agent_id,
        EventPayload::TaskStarted(TaskStarted {
            task_id: "task-cargo-binary-fix".to_string(),
            title: "Execute test suite for project".to_string(),
            description: Some("Run unit tests with cargo test".to_string()),
            parent_task_id: None,
            session_id: session_id.to_string(),
            timestamp: start_time,
        }),
    ));

    // Failing tool invocation (cargo test --bin non_existent)
    let inv1_id = Uuid::new_v4();
    events.push(Event::new(
        session_id,
        agent_id,
        EventPayload::ToolInvoked(ToolInvoked {
            invocation_id: inv1_id,
            tool_name: "run_command".to_string(),
            input: serde_json::json!({ "command": "cargo test --bin non_existent" }),
            session_id: session_id.to_string(),
            timestamp: start_time,
        }),
    ));

    events.push(Event::new(
        session_id,
        agent_id,
        EventPayload::ToolResultReceived(ToolResultReceived {
            invocation_id: inv1_id,
            tool_name: "run_command".to_string(),
            result: serde_json::json!({ "stderr": "error: no bin target `non_existent` in package" }),
            is_error: true,
            duration_ms: Some(500),
            timestamp: start_time,
        }),
    ));

    events.push(Event::new(
        session_id,
        agent_id,
        EventPayload::ErrorObserved(ErrorObserved {
            error_type: "TargetNotFound".to_string(),
            message: "error: no bin target `non_existent` in package".to_string(),
            severity: "high".to_string(),
            context: None,
            stack_trace: None,
            timestamp: start_time,
        }),
    ));

    // Correcting tool invocation (cargo test --bin strata)
    let inv2_id = Uuid::new_v4();
    events.push(Event::new(
        session_id,
        agent_id,
        EventPayload::ToolInvoked(ToolInvoked {
            invocation_id: inv2_id,
            tool_name: "run_command".to_string(),
            input: serde_json::json!({ "command": "cargo test --bin strata" }),
            session_id: session_id.to_string(),
            timestamp: start_time,
        }),
    ));

    events.push(Event::new(
        session_id,
        agent_id,
        EventPayload::ToolResultReceived(ToolResultReceived {
            invocation_id: inv2_id,
            tool_name: "run_command".to_string(),
            result: serde_json::json!({ "stdout": "test result: ok. 16 passed; 0 failed" }),
            is_error: false,
            duration_ms: Some(850),
            timestamp: start_time,
        }),
    ));

    events.push(Event::new(
        session_id,
        agent_id,
        EventPayload::TaskCompleted(TaskCompleted {
            task_id: "task-cargo-binary-fix".to_string(),
            success: true,
            outcome_summary: "Resolved target name error and successfully executed all 16 tests"
                .to_string(),
            evaluation: None,
            timestamp: start_time,
        }),
    ));

    events.push(Event::new(
        session_id,
        agent_id,
        EventPayload::SessionEnded(SessionEnded {
            session_id: session_id.to_string(),
            agent_id: agent_id.to_string(),
            final_state: Some("completed".to_string()),
            reason: Some("Tests passing".to_string()),
            summary: Some("Recovered from invalid target specification".to_string()),
            timestamp: start_time,
        }),
    ));

    for ev in &events {
        store_arc.insert_event(ev)?;
    }

    // Record a known failure pattern
    let mut failure = FailurePattern::new(
        "fail:cargo:non_existent",
        "CargoInvalidBinaryTarget",
        "Attempted to execute cargo test with an unconfigured binary target name",
        "Check Cargo.toml [[bin]] section or omit --bin to run default test harness",
    );
    failure.trigger_condition = "error: no bin target".to_string();
    failure.severity = FailureSeverity::High;
    store_arc.upsert_failure_pattern(&failure)?;

    // Record a semantic memory record and apply explicit positive feedback
    let mem_valid = MemoryRecord::new(
        MemoryType::Semantic,
        "SQLite WAL mode enables concurrent reader transactions without blocking writer synchronization.",
        Scope::Global,
    )
    .with_summary("SQLite WAL Concurrency")
    .with_importance(0.60)
    .with_confidence(0.80);

    store_arc.insert_or_update_memory(&mem_valid)?;
    let fb_pos =
        MemoryFeedback::positive(mem_valid.id).with_comment("Verified in stress benchmarks");
    store_arc.record_memory_feedback(&fb_pos)?;

    let updated_valid = store_arc
        .get_memory(&mem_valid.id)?
        .expect("valid memory must exist");
    if updated_valid.confidence <= 0.80 {
        bail!(
            "Positive feedback should increase memory confidence above initial 0.80, got {}",
            updated_valid.confidence
        );
    }
    if updated_valid.importance <= 0.60 {
        bail!(
            "Positive feedback should increase memory importance above initial 0.60, got {}",
            updated_valid.importance
        );
    }

    // Record an erroneous memory record and apply explicit negative feedback
    let mem_bad = MemoryRecord::new(
        MemoryType::Semantic,
        "Embedded PostgreSQL cluster is used for local-first desktop persistence.",
        Scope::Global,
    )
    .with_summary("Postgres Database Engine")
    .with_importance(0.50)
    .with_confidence(0.50);

    store_arc.insert_or_update_memory(&mem_bad)?;
    let fb_neg = MemoryFeedback::negative(
        mem_bad.id,
        Some("Strata uses SQLite embedded, not PostgreSQL".to_string()),
    );
    store_arc.record_memory_feedback(&fb_neg)?;

    let updated_bad = store_arc
        .get_memory(&mem_bad.id)?
        .expect("bad memory must exist");
    if updated_bad.confidence >= 0.50 {
        bail!(
            "Negative feedback should decrease memory confidence below initial 0.50, got {}",
            updated_bad.confidence
        );
    }
    println!("    ✓ Implicit signals and explicit feedback recorded cleanly.");

    // ========================================================================
    // Part B: Mining DPO Preference Pairs (Failure Trajectory vs Resolution)
    // ========================================================================
    println!("  [Step B] Mining DPO preference pairs...");
    let miner = PreferenceMiner::new(Arc::clone(&store_arc));

    let dpo_pairs = miner.mine_dpo_pairs(None)?;
    println!("    • Total DPO Pairs Mined: {}", dpo_pairs.len());

    if dpo_pairs.is_empty() {
        bail!("Expected at least 1 DPO preference pair to be mined");
    }

    // Verify implicit command fix pair was mined
    let implicit_pair = dpo_pairs
        .iter()
        .find(|p| p.rejected.contains("non_existent") && p.chosen.contains("strata"));

    if implicit_pair.is_none() {
        bail!("Preference miner failed to extract implicit CommandFix DPO pair from event stream");
    }
    let pair = implicit_pair.unwrap();
    println!("    • Found CommandFix DPO Pair: Prompt='{}'", pair.prompt);

    // Verify failure pattern pair was mined
    let failure_pair = dpo_pairs
        .iter()
        .find(|p| p.chosen.contains("Check Cargo.toml"));
    if failure_pair.is_none() {
        bail!("Preference miner failed to extract DPO pair from failure pattern mitigation");
    }

    // Verify explicit feedback pair was mined
    let feedback_pair = dpo_pairs
        .iter()
        .find(|p| p.rejected.contains("PostgreSQL") && p.chosen.contains("Strata uses SQLite"));
    if feedback_pair.is_none() {
        bail!("Preference miner failed to extract DPO pair from explicit negative feedback correction");
    }

    // Validate DPO JSON format
    let dpo_jsonl = miner.export(ExportFormat::Dpo, None)?;
    for line in dpo_jsonl.lines() {
        let parsed: DpoPair = serde_json::from_str(line)?;
        if parsed.prompt.is_empty() || parsed.chosen.is_empty() || parsed.rejected.is_empty() {
            bail!("DPO pair contains empty fields: {:?}", parsed);
        }
    }
    println!("    ✓ DPO preference pairs validated and exported cleanly.");

    // ========================================================================
    // Part C: Mining KTO Samples & SFT Procedural Skills
    // ========================================================================
    println!("  [Step C] Mining KTO samples and SFT procedural skills...");

    // Store procedural skill
    let skill = ProceduralSkill {
        id: Uuid::new_v4(),
        name: "fix_cargo_target_and_test".to_string(),
        project: None,
        description: "Diagnose invalid binary target error and execute tests on valid crate binary"
            .to_string(),
        preconditions: vec![
            "Rust project contains Cargo.toml".to_string(),
            "Cargo command-line tool available".to_string(),
        ],
        postconditions: vec!["Tests executed with result summary".to_string()],
        parameters: vec![ParameterDef::new(
            "bin_name",
            "string",
            "Name of target binary defined in Cargo.toml",
        )],
        steps: vec![
            ProceduralStep::new(
                1,
                "view_file",
                "Inspect Cargo.toml [[bin]] sections to identify configured binary target names",
                serde_json::Value::Null,
            )
            .with_expected_result("List of valid binary targets"),
            ProceduralStep::new(
                2,
                "run_command",
                "Execute cargo test --bin <valid_target> to run test suite",
                serde_json::Value::Null,
            )
            .with_expected_result("Passage of all unit test assertions"),
        ],
        examples: vec![],
        success_rate: 0.95,
        importance: 0.90,
        created_at: Utc::now(),
        last_used_at: None,
        usage_count: 5,
        tags: vec![
            "cargo".to_string(),
            "rust".to_string(),
            "testing".to_string(),
        ],
    };

    store_arc.insert_or_update_procedural_skill(&skill)?;

    // Mine KTO samples
    let kto_samples = miner.mine_kto_samples(None)?;
    println!("    • Total KTO Samples: {}", kto_samples.len());

    let has_positive = kto_samples.iter().any(|s| s.label);
    let has_negative = kto_samples.iter().any(|s| !s.label);

    if !has_positive || !has_negative {
        bail!(
            "KTO mining should produce both positive (true) and negative (false) alignment samples"
        );
    }

    let kto_jsonl = miner.export(ExportFormat::Kto, None)?;
    for line in kto_jsonl.lines() {
        let parsed: KtoSample = serde_json::from_str(line)?;
        if parsed.prompt.is_empty() || parsed.completion.is_empty() {
            bail!(
                "KTO sample contains empty prompt or completion: {:?}",
                parsed
            );
        }
    }

    // Mine SFT samples
    let sft_samples = miner.mine_sft_samples()?;
    println!("    • Total SFT Samples: {}", sft_samples.len());

    if sft_samples.is_empty() {
        bail!("Expected at least 1 SFT sample from procedural skills");
    }

    let sft_skill = sft_samples
        .iter()
        .find(|s| s.instruction.contains("fix_cargo_target_and_test"));
    if sft_skill.is_none() {
        bail!("SFT miner failed to extract procedural skill workflow");
    }

    let sft_jsonl = miner.export(ExportFormat::Sft, None)?;
    for line in sft_jsonl.lines() {
        let parsed: SftSample = serde_json::from_str(line)?;
        if parsed.instruction.is_empty() || parsed.output.is_empty() {
            bail!("SFT sample contains empty instruction or output");
        }
    }
    println!("    ✓ KTO samples and SFT procedural skills mined and validated.");

    // ========================================================================
    // Part D: Multi-Host Context Compilation Respecting Token Budget
    // ========================================================================
    println!("  [Step D] Compiling multi-host instructions with token budgeting...");

    let temp_dir = std::env::temp_dir().join(format!("strata-eval-align-{}", Uuid::new_v4()));
    fs::create_dir_all(&temp_dir)?;

    // Pre-populate existing instruction files with custom user rules to test non-destructive injection
    let claude_md = temp_dir.join("CLAUDE.md");
    fs::write(
        &claude_md,
        "# Custom Claude Rules\n- Always run cargo fmt before committing.\n",
    )?;

    let agents_md = temp_dir.join("AGENTS.md");
    fs::write(
        &agents_md,
        "# Custom Codex Rules\n- Radical simplicity and lean code.\n",
    )?;

    let compiler = MultiHostCompiler::new(Arc::clone(&store_arc));

    // Test token budgeting: budget = 400 tokens
    let token_budget = 400;
    let report = compiler.compile_workspace(
        &temp_dir,
        &["cursor", "claude", "codex", "gemini"],
        token_budget,
    )?;

    println!(
        "    • Compiled Tokens: ~{} / Budget: {}",
        report.total_tokens, token_budget
    );
    println!("    • Targets Updated: {}", report.target_hosts.len());

    if report.target_hosts.len() != 4 {
        bail!(
            "Expected 4 target hosts to be compiled, got {}",
            report.target_hosts.len()
        );
    }

    if report.total_tokens > token_budget {
        bail!(
            "Compiled context (~{} tokens) exceeded token budget ({} tokens)",
            report.total_tokens,
            token_budget
        );
    }

    // Verify 1: Cursor (.cursor/rules/strata.mdc)
    let cursor_file = temp_dir.join(".cursor").join("rules").join("strata.mdc");
    if !cursor_file.exists() {
        bail!(
            "Cursor instruction file was not created at {}",
            cursor_file.display()
        );
    }
    let cursor_content = fs::read_to_string(&cursor_file)?;
    if !cursor_content.contains("<!-- STRATA_MEMORY_START -->")
        || !cursor_content.contains("<!-- STRATA_MEMORY_END -->")
    {
        bail!("Cursor file missing Strata memory markers");
    }

    // Verify 2: Claude Code (CLAUDE.md) preserves user rules outside markers
    let claude_content = fs::read_to_string(&claude_md)?;
    if !claude_content.contains("Always run cargo fmt before committing") {
        bail!("Claude Code compiler wiped user custom rules outside memory markers!");
    }
    if !claude_content.contains("<!-- STRATA_MEMORY_START -->") {
        bail!("Claude Code missing Strata memory marker block");
    }
    if !claude_content.contains("CargoInvalidBinaryTarget") {
        bail!("Claude Code compiled block missing known failure anti-pattern");
    }

    // Verify 3: Codex (AGENTS.md) preserves user rules outside markers
    let agents_content = fs::read_to_string(&agents_md)?;
    if !agents_content.contains("Radical simplicity and lean code") {
        bail!("Codex compiler wiped user custom rules outside memory markers!");
    }
    if !agents_content.contains("<!-- STRATA_MEMORY_START -->") {
        bail!("Codex missing Strata memory marker block");
    }

    // Verify 4: Gemini (.gemini/GEMINI.md)
    let gemini_file = temp_dir.join(".gemini").join("GEMINI.md");
    if !gemini_file.exists() {
        bail!(
            "Gemini instruction file was not created at {}",
            gemini_file.display()
        );
    }
    let gemini_content = fs::read_to_string(&gemini_file)?;
    if !gemini_content.contains("<!-- STRATA_MEMORY_START -->") {
        bail!("Gemini file missing Strata memory markers");
    }

    // Cleanup temp directory
    let _ = fs::remove_dir_all(&temp_dir);

    println!(
        "  ✓ Multi-host context compilation and token-budget alignment verified successfully!"
    );
    println!("  ✓ Cognitive feedback, preference mining & alignment eval scenario PASSED cleanly.");
    Ok(())
}
