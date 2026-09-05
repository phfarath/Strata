use anyhow::{bail, Result};
use chrono::Utc;
use strata_core::events::{
    ErrorObserved, Event, EventPayload, SessionEnded, SessionStarted, TaskCompleted, TaskStarted,
    ToolInvoked, ToolResultReceived,
};
use strata_memory::{ConsolidationPipeline, MockEmbeddingProvider, SqliteStore};
use strata_reasoning::{
    mock::MockReasoningEngine,
    prompts::{DistillationOutput, EpisodicMemoryItem, ProceduralSkill, ProceduralStep},
};
use uuid::Uuid;

/// Scenario 5: Procedural Skill Distillation from Tool Failure & Recovery Experience
/// Evaluates:
/// 1. An agent encounters a build error (`cargo check` failure), corrects dependencies with `cargo add`, and verifies clean build.
/// 2. The multi-stage consolidation pipeline extracts an episodic memory and distills a reusable `ProceduralSkill`.
/// 3. The procedural skill preserves ordered execution steps and preconditions.
/// 4. Both episodic memory and procedural skill are stored in SQLite.
pub async fn run_procedural_skill_distillation_scenario() -> Result<()> {
    println!("\n▶ Running Eval Scenario: Procedural Skill Distillation from Failure Recovery");

    // 1. Setup in-memory SQLite store and pipeline
    let store = SqliteStore::open_in_memory()?;
    let embedder = MockEmbeddingProvider::default();
    let pipeline = ConsolidationPipeline::with_default_config();

    let session_id = "sess-recovery-workflow-42";
    let agent_id = "agent-strata-engineer";

    // 2. Build Event Stream simulating Tool Failure + Recovery Sequence
    let start_time = Utc::now();
    let mut events = Vec::new();

    // Event 0: SessionStarted
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

    // Event 1: TaskStarted
    events.push(Event::new(
        session_id,
        agent_id,
        EventPayload::TaskStarted(TaskStarted {
            task_id: "task-cargo-fix-01".to_string(),
            title: "Resolve missing serde_json dependency".to_string(),
            description: Some(
                "Diagnose compiler errors and add missing dependency with derive features"
                    .to_string(),
            ),
            parent_task_id: None,
            session_id: session_id.to_string(),
            timestamp: start_time,
        }),
    ));

    // Event 2: ToolInvoked (cargo check)
    let inv1_id = Uuid::new_v4();
    events.push(Event::new(
        session_id,
        agent_id,
        EventPayload::ToolInvoked(ToolInvoked {
            invocation_id: inv1_id,
            tool_name: "run_command".to_string(),
            input: serde_json::json!({ "command": "cargo check" }),
            session_id: session_id.to_string(),
            timestamp: start_time,
        }),
    ));

    // Event 3: ToolResultReceived (Error)
    events.push(Event::new(
        session_id,
        agent_id,
        EventPayload::ToolResultReceived(ToolResultReceived {
            invocation_id: inv1_id,
            tool_name: "run_command".to_string(),
            result: serde_json::json!({ "stderr": "error[E0432]: unresolved import `serde_json`" }),
            is_error: true,
            duration_ms: Some(1200),
            timestamp: start_time,
        }),
    ));

    // Event 4: ErrorObserved
    events.push(Event::new(
        session_id,
        agent_id,
        EventPayload::ErrorObserved(ErrorObserved {
            error_type: "MissingDependencyError".to_string(),
            message: "error[E0432]: unresolved import serde_json".to_string(),
            severity: "high".to_string(),
            context: None,
            stack_trace: None,
            timestamp: start_time,
        }),
    ));

    // Event 5: ToolInvoked (cargo add)
    let inv2_id = Uuid::new_v4();
    events.push(Event::new(
        session_id,
        agent_id,
        EventPayload::ToolInvoked(ToolInvoked {
            invocation_id: inv2_id,
            tool_name: "run_command".to_string(),
            input: serde_json::json!({ "command": "cargo add serde_json --features derive" }),
            session_id: session_id.to_string(),
            timestamp: start_time,
        }),
    ));

    // Event 6: ToolResultReceived (Success)
    events.push(Event::new(
        session_id,
        agent_id,
        EventPayload::ToolResultReceived(ToolResultReceived {
            invocation_id: inv2_id,
            tool_name: "run_command".to_string(),
            result: serde_json::json!({ "stdout": "Adding serde_json v1.0 to dependencies" }),
            is_error: false,
            duration_ms: Some(850),
            timestamp: start_time,
        }),
    ));

    // Event 7: ToolInvoked (cargo check)
    let inv3_id = Uuid::new_v4();
    events.push(Event::new(
        session_id,
        agent_id,
        EventPayload::ToolInvoked(ToolInvoked {
            invocation_id: inv3_id,
            tool_name: "run_command".to_string(),
            input: serde_json::json!({ "command": "cargo check" }),
            session_id: session_id.to_string(),
            timestamp: start_time,
        }),
    ));

    // Event 8: ToolResultReceived (Success)
    events.push(Event::new(
        session_id,
        agent_id,
        EventPayload::ToolResultReceived(ToolResultReceived {
            invocation_id: inv3_id,
            tool_name: "run_command".to_string(),
            result: serde_json::json!({ "stdout": "Finished dev profile [unoptimized + debuginfo] target(s) in 0.45s" }),
            is_error: false,
            duration_ms: Some(450),
            timestamp: start_time,
        }),
    ));

    // Event 9: TaskCompleted (Success)
    events.push(Event::new(
        session_id,
        agent_id,
        EventPayload::TaskCompleted(TaskCompleted {
            task_id: "task-cargo-fix-01".to_string(),
            success: true,
            outcome_summary: "Cargo dependency resolved and compilation succeeded".to_string(),
            evaluation: None,
            timestamp: start_time,
        }),
    ));

    // Event 10: SessionEnded
    events.push(Event::new(
        session_id,
        agent_id,
        EventPayload::SessionEnded(SessionEnded {
            session_id: session_id.to_string(),
            agent_id: agent_id.to_string(),
            final_state: Some("completed".to_string()),
            reason: Some("Task finished successfully".to_string()),
            summary: Some("Recovered from missing dependency build error".to_string()),
            timestamp: start_time,
        }),
    ));

    // Append events to store
    for ev in &events {
        store.insert_event(ev)?;
    }

    // 3. Configure MockReasoningEngine with structured DistillationOutput
    let reasoning_engine = MockReasoningEngine::new();
    let structured_distillation = DistillationOutput {
        episodic_memories: vec![EpisodicMemoryItem {
            summary: "Encountered missing serde_json compile error, added crate with derive flag, and confirmed clean cargo check build.".to_string(),
            content: "Encountered missing serde_json compile error, added crate with derive flag, and confirmed clean cargo check build.".to_string(),
            importance: 0.85,
            tags: vec!["cargo".to_string(), "build-recovery".to_string(), "rust".to_string()],
        }],
        semantic_facts: vec![],
        procedural_skills: vec![ProceduralSkill {
            name: "resolve_rust_missing_dependency".to_string(),
            description: "Diagnose unresolved crate import errors and install required dependency with appropriate feature flags".to_string(),
            trigger_conditions: vec!["error[E0432]".to_string(), "unresolved import".to_string()],
            preconditions: vec![
                "Rust workspace contains Cargo.toml".to_string(),
                "cargo command-line tool is installed on PATH".to_string(),
            ],
            steps: vec![
                ProceduralStep {
                    step_number: 1,
                    action: "Run cargo check to capture exact missing crate name and symbol".to_string(),
                    tool_name: Some("run_command".to_string()),
                    expected_outcome: Some("Compiler diagnostic output".to_string()),
                },
                ProceduralStep {
                    step_number: 2,
                    action: "Execute cargo add <crate> --features <features> to update manifest".to_string(),
                    tool_name: Some("run_command".to_string()),
                    expected_outcome: Some("Manifest updated".to_string()),
                },
                ProceduralStep {
                    step_number: 3,
                    action: "Re-run cargo check to verify clean compilation without diagnostics".to_string(),
                    tool_name: Some("run_command".to_string()),
                    expected_outcome: Some("Finished dev profile".to_string()),
                },
            ],
            error_recovery: Some("If feature flag is missing, check crates.io documentation".to_string()),
            importance: 0.90,
            tags: vec!["rust".to_string(), "cargo".to_string(), "troubleshooting".to_string()],
        }],
        negative_patterns: vec![],
    };

    reasoning_engine
        .push_distillation_output(&structured_distillation)
        .await;

    // 4. Run consolidation pipeline
    let result = pipeline
        .run_pipeline(&store, &embedder, &events, Some(&reasoning_engine))
        .await?;

    println!("  [Consolidation Output]");
    println!("    • Events Processed:    {}", result.events_processed);
    println!(
        "    • Episodic Memories:   {}",
        result.episodic_memories.len()
    );
    println!(
        "    • Procedural Skills:   {}",
        result.procedural_skills.len()
    );

    if result.episodic_memories.is_empty() {
        bail!("Expected at least 1 episodic memory to be created");
    }

    if result.procedural_skills.is_empty() {
        bail!("Expected at least 1 procedural skill to be distilled");
    }

    // 5. Verify Procedural Skill in SQLite
    let distilled_skill = &result.procedural_skills[0];
    println!("\n  [Distilled Procedural Skill]");
    println!("    • Skill Name:     {}", distilled_skill.name);
    println!("    • Description:    {}", distilled_skill.description);
    println!("    • Preconditions:  {:?}", distilled_skill.preconditions);
    println!("    • Step Count:     {}", distilled_skill.steps.len());

    if distilled_skill.steps.len() != 3 {
        bail!(
            "Procedural skill should have exactly 3 steps, found: {}",
            distilled_skill.steps.len()
        );
    }

    if distilled_skill.preconditions.is_empty() {
        bail!("Procedural skill should have preconditions specified");
    }

    // Query SQLite to verify persistence
    let retrieved_skill = store
        .get_procedural_skill(&distilled_skill.id)?
        .expect("Procedural skill must exist in SQLite store");

    if retrieved_skill.name != distilled_skill.name {
        bail!(
            "Retrieved skill name mismatch: {} vs {}",
            retrieved_skill.name,
            distilled_skill.name
        );
    }

    println!(
        "  ✓ Procedural skill distillation and failure-recovery learning verified successfully!"
    );
    Ok(())
}
