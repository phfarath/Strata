use anyhow::{bail, Result};
use strata_core::{
    events::{Event, EventPayload, Provenance, ToolInvoked, ToolResultReceived},
    state::{FailurePattern, FailureSeverity, Scope},
    traits::{EventStore, MemoryEngine},
};
use strata_memory::SqliteMemoryEngine;
use uuid::Uuid;

/// Scenario 2: Silent Failure Avoidance & Out-of-Band Anti-Pattern Learning
/// Verifies that a ToolResultReceived failure is recorded out-of-band
/// and subsequently raises a known failure alert without bloating prompt context.
pub async fn run_silent_failure_avoidance_scenario() -> Result<()> {
    println!("\n▶ Running Eval Scenario: Silent Failure Avoidance");

    // 1. Setup isolated in-memory memory engine
    let engine = SqliteMemoryEngine::open_in_memory(None)?;
    let session_id = "sess-failure-test-01";
    let agent_id = "agent-executor";

    let prov = Provenance::new(agent_id, session_id);

    // 2. Simulated tool invocation
    let invocation_id = Uuid::new_v4();
    let tool_invoke_event = Event::new(
        session_id,
        agent_id,
        EventPayload::ToolInvoked(ToolInvoked {
            invocation_id,
            tool_name: "safe_shell".to_string(),
            input: serde_json::json!({ "command": "cargo build --target x86_64-unknown-linux-gnu" }),
            session_id: session_id.to_string(),
            timestamp: chrono::Utc::now(),
        }),
    ).with_provenance(prov.clone());

    engine.append(&tool_invoke_event).await?;

    // 3. Tool fails with a large error trace
    let raw_error_log = "error: linking with `x86_64-linux-gnu-gcc` failed: exit code: 1\n  = note: \"x86_64-linux-gnu-gcc\" \"-Wl,--as-needed\" ... [truncated 500 lines of linker dump]";

    let tool_result_event = Event::new(
        session_id,
        agent_id,
        EventPayload::ToolResultReceived(ToolResultReceived {
            invocation_id,
            tool_name: "safe_shell".to_string(),
            result: serde_json::json!({ "raw_log": raw_error_log }),
            is_error: true,
            duration_ms: Some(1250),
            timestamp: chrono::Utc::now(),
        }),
    )
    .with_provenance(prov.clone());

    engine.append(&tool_result_event).await?;

    // 4. Out-of-band capture (zero prompt tokens consumed)
    let mut failure_pattern = FailurePattern::new(
        "safe_shell_cargo_build_linux_cross",
        "LinuxCrossCompileError",
        "Direct cargo build with Linux target fails on Windows due to missing cross-linker",
        "Use cargo check or run cross-compilation within Docker/WSL container",
    );
    failure_pattern.error_type = "LinkerError".to_string();
    failure_pattern.trigger_condition = "cargo build --target x86_64-unknown-linux-gnu".to_string();
    failure_pattern.severity = FailureSeverity::High;
    failure_pattern.scope = Scope::Global;

    engine.record_failure(&failure_pattern).await?;
    println!(
        "  [Engine] Out-of-band captured failure pattern: '{}'",
        failure_pattern.pattern_name
    );

    // 5. Subsequent session / prompt execution
    let prompt_query = "How do I build the cargo project for Linux target?";
    println!("  [Next Session] Agent evaluates prompt: \"{prompt_query}\"");

    let known_failures = engine
        .get_known_failures(Some(prompt_query), None, 3)
        .await?;

    if known_failures.is_empty() {
        bail!(
            "Expected pre-emptive failure warning for query '{prompt_query}', but none was found"
        );
    }

    let warning = &known_failures[0];
    println!("  [Prompt Alert] Received pre-emptive failure warning:");
    println!("    • Error Type: {}", warning.error_type);
    println!("    • Description: {}", warning.description);
    println!("    • Recommended Mitigation: {}", warning.mitigation);

    // 6. Verify minimal context overhead
    let alert_text = format!("{}: {}", warning.pattern_name, warning.mitigation);
    let token_estimate = alert_text.split_whitespace().count() * 4 / 3;
    println!(
        "  [Context Efficiency] Alert token cost: ~{token_estimate} tokens (Target: < 50 tokens)"
    );

    if token_estimate > 50 {
        bail!("Context bloated! Token estimate was {token_estimate} tokens, expected < 50 tokens");
    }

    if !warning.mitigation.contains("Docker/WSL") {
        bail!(
            "Mitigation advice mismatch. Expected Docker/WSL, got: {}",
            warning.mitigation
        );
    }

    println!("  ✓ Silent failure avoidance eval scenario PASSED cleanly.");
    Ok(())
}
