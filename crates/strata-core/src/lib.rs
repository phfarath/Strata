pub mod a2a;
pub mod blackboard;
pub mod config;
pub mod errors;
pub mod events;
pub mod paths;
pub mod schemas;
pub mod state;
pub mod traits;

// Re-exports for convenience
pub use a2a::{
    A2aMessage, AgentCard, AgentPresence, DomainAuthority, IpcEvent, IpcMessage,
    LeaseAcquireResult, ResourceLease, TransactiveQuery, TransactiveResponse,
};
pub use blackboard::{TaskArtifact, TaskFilter, TaskRecord, TaskStatus};
pub use config::StrataConfig;
pub use errors::StrataError;
pub use events::{
    DataClassification, ErrorObserved, Event, EventId, EventPayload, MemoryConsolidated,
    MemoryWritten, ObservationReceived, Provenance, RetentionPolicy, SessionEnded, SessionStarted,
    TaskCompleted, TaskStarted, ToolInvoked, ToolResultReceived,
};
pub use paths::{canonical_resource_id, canonical_resource_id_with_root};
pub use schemas::{
    BranchMergeReport, CodeAnchor, ContextBudgetConfig, DecayConfig, DecayMetrics, EpisodeRecord,
    EpisodeReplay, EpisodeStep, EpisodicMemory, EvidenceRef, ExportFormat, FactDependency,
    FactStatus, FeedbackEvent, FeedbackRating, HippocampalConsolidationResult, HostTargetConfig,
    ImplicitSignal, JtmsAuditRow, KtoSample, MemoryFeedback, ParameterDef, PreferencePair,
    ProceduralExample, ProceduralSkill, ProceduralStep, SemanticFact, SftSample, SignalKind,
    SignalScores, SymbolType, SyncConfig, SyncDelta, SyncReport,
};
pub use state::{
    DigestOutput, FailurePattern, FailureSeverity, HumanApprovalRecord, MemoryHandle, MemoryRecord,
    MemoryTier, MemoryType, OutboxEntry, OutboxStatus, ProvenanceBlock, Scope, SessionState,
    SessionStatus,
};

pub use traits::{
    ChatMessage, EventStore, MemoryEngine, PromptContext, ReasoningEngine, ReasoningOutput, Role,
    TokenUsage, Tool, ToolCall, ToolDefinition, ToolResult,
};

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use uuid::Uuid;

    #[test]
    fn test_event_creation_and_serialization() {
        let payload = EventPayload::SessionStarted(SessionStarted {
            session_id: "sess-123".to_string(),
            agent_id: "agent-007".to_string(),
            organization_id: Some("org-acme".to_string()),
            environment: serde_json::json!({"os": "windows"}),
            timestamp: Utc::now(),
        });

        let event = Event::new("sess-123", "agent-007", payload)
            .with_classification(DataClassification::Internal)
            .with_retention(RetentionPolicy::Permanent);

        let json = serde_json::to_string(&event).expect("serialize event");
        let deserialized: Event = serde_json::from_str(&json).expect("deserialize event");

        assert_eq!(event.session_id, deserialized.session_id);
        assert_eq!(event.agent_id, deserialized.agent_id);
        assert_eq!(event.classification, deserialized.classification);
        assert_eq!(event.retention, deserialized.retention);
        assert_eq!(deserialized.payload.event_type(), "SessionStarted");
    }

    #[test]
    fn test_memory_record_and_handle() {
        let record = MemoryRecord::new(
            MemoryType::Semantic,
            "Rust ensures memory safety without GC",
            Scope::Project("strata".to_string()),
        )
        .with_summary("Rust memory safety")
        .with_importance(0.9)
        .with_tags(vec!["rust".to_string(), "systems".to_string()]);

        let handle = record.to_handle(Some(0.95));
        assert_eq!(handle.id, record.id);
        assert_eq!(handle.title, "Rust memory safety");
        assert_eq!(handle.memory_type, MemoryType::Semantic);
        assert_eq!(handle.relevance_score, Some(0.95));
    }

    #[test]
    fn test_failure_pattern_serialization() {
        let mut failure = FailurePattern::new(
            "sig-timeout-tool-fetch",
            "FetchTimeout",
            "HTTP tool fetch timed out after 30s",
            "Increase timeout or retry with exponential backoff",
        );
        failure.severity = FailureSeverity::High;
        failure.scope = Scope::Organization("org-acme".to_string());

        let json = serde_json::to_string(&failure).expect("serialize failure");
        let deserialized: FailurePattern =
            serde_json::from_str(&json).expect("deserialize failure");

        assert_eq!(failure.signature, deserialized.signature);
        assert_eq!(failure.severity, deserialized.severity);
        assert_eq!(failure.scope, deserialized.scope);
    }

    #[test]
    fn test_scope_parsing_and_compatibility() {
        let global = Scope::Global;
        let p1 = "project:repo-alpha".parse::<Scope>().unwrap();
        let p2 = "project:repo-beta".parse::<Scope>().unwrap();
        let p1_again = Scope::Project("repo-alpha".to_string());

        assert!(global.is_compatible(&p1));
        assert!(p1.is_compatible(&global));
        assert!(p1.is_compatible(&p1_again));
        assert!(!p1.is_compatible(&p2));
    }

    #[test]
    fn test_phase2_schemas_serialization() {
        let now = Utc::now();
        let ep = EpisodicMemory::new("sess-p2", "assistant", "Implemented new features", now, now)
            .with_project("strata")
            .with_goals(vec!["Goal 1".to_string()])
            .with_obstacles(vec!["Obstacle 1".to_string()])
            .with_outcomes(vec!["Success".to_string()])
            .with_tools(vec!["cargo_build".to_string()])
            .with_files(vec!["src/main.rs".to_string()])
            .with_signals(SignalScores {
                success: 0.9,
                frustration: 0.1,
                novelty: 0.8,
                importance: 0.85,
            });

        let json_ep = serde_json::to_string(&ep).expect("serialize episodic memory");
        let de_ep: EpisodicMemory =
            serde_json::from_str(&json_ep).expect("deserialize episodic memory");
        assert_eq!(ep.id, de_ep.id);
        assert_eq!(ep.session_id, de_ep.session_id);
        assert_eq!(ep.signals.success, de_ep.signals.success);

        let mut fact = SemanticFact::new(
            "FTS5 Porter stemmer normalizes inflected words",
            "search",
            Scope::Project("strata".to_string()),
        )
        .with_importance(0.8)
        .with_confidence(0.95)
        .with_evidence(vec![EvidenceRef::new("event", "ev-100", 0.9)]);

        assert_eq!(fact.status, FactStatus::Active);
        assert_eq!(fact.version, 1);

        let fact_json = serde_json::to_string(&fact).expect("serialize fact");
        let de_fact: SemanticFact = serde_json::from_str(&fact_json).expect("deserialize fact");
        assert_eq!(fact.id, de_fact.id);

        let next_id = uuid::Uuid::new_v4();
        fact.deprecate_and_replace(next_id);
        assert_eq!(fact.status, FactStatus::Deprecated);
        assert_eq!(fact.replaced_by, Some(next_id));

        let skill = ProceduralSkill::new("compile_workspace", "Build all workspace crates")
            .with_preconditions(vec!["Rust toolchain installed".to_string()])
            .with_postconditions(vec!["Binaries compiled in target/".to_string()])
            .with_parameters(vec![ParameterDef::new(
                "release",
                "bool",
                "Build with optimizations",
            )])
            .with_steps(vec![ProceduralStep::new(
                1,
                "cargo",
                "build",
                serde_json::json!({"release": true}),
            )])
            .with_examples(vec![ProceduralExample::new("sess-1", "Compiled in 2.3s")]);

        let skill_json = serde_json::to_string(&skill).expect("serialize skill");
        let de_skill: ProceduralSkill =
            serde_json::from_str(&skill_json).expect("deserialize skill");
        assert_eq!(skill.name, de_skill.name);
        assert_eq!(de_skill.steps.len(), 1);

        let decay_config = DecayConfig::default();
        assert_eq!(decay_config.d, 0.5);
        assert_eq!(decay_config.s0, 24.0);
    }

    #[test]
    fn test_track2_sync_schemas() {
        let delta = SyncDelta::new(
            "ws-123",
            42,
            "fact",
            serde_json::json!({"statement": "SQLite is durable"}),
            "hash-abc",
        );
        assert_eq!(delta.workspace_id, "ws-123");
        assert_eq!(delta.seq, 42);
        assert_eq!(delta.kind, "fact");
        assert!(!delta.synced);

        let delta_json = serde_json::to_string(&delta).expect("serialize delta");
        let de_delta: SyncDelta = serde_json::from_str(&delta_json).expect("deserialize delta");
        assert_eq!(delta.id, de_delta.id);
        assert_eq!(delta.version_hash, de_delta.version_hash);

        let fb = MemoryFeedback::positive(delta.id).with_comment("Very relevant");
        assert_eq!(fb.rating, "positive");
        assert_eq!(fb.score, Some(1.0));
        assert_eq!(fb.comment.as_deref(), Some("Very relevant"));

        let fb_json = serde_json::to_string(&fb).expect("serialize feedback");
        let de_fb: MemoryFeedback = serde_json::from_str(&fb_json).expect("deserialize feedback");
        assert_eq!(fb.memory_id, de_fb.memory_id);

        let config = SyncConfig::new("ws-test")
            .with_endpoint("https://api.strata.dev/sync")
            .with_token("secret-token")
            .with_batch_size(50);
        assert_eq!(config.workspace_id, "ws-test");
        assert_eq!(
            config.endpoint.as_deref(),
            Some("https://api.strata.dev/sync")
        );
        assert_eq!(config.batch_size, 50);

        let report = SyncReport {
            pushed_count: 10,
            pulled_count: 5,
            conflicts_resolved: 1,
            last_seq: 42,
            errors: vec![],
        };
        let report_json = serde_json::to_string(&report).expect("serialize report");
        let de_report: SyncReport = serde_json::from_str(&report_json).expect("deserialize report");
        assert_eq!(report.pushed_count, de_report.pushed_count);
        assert_eq!(report.conflicts_resolved, de_report.conflicts_resolved);
    }

    #[test]
    fn test_track3_schemas_serialization() {
        let signal = ImplicitSignal::new(SignalKind::ToolLoop, "sess-test-3", "agent-beta")
            .with_tool_name("file_search")
            .with_file_path("src/lib.rs")
            .with_extra("Loop detected 4 times");

        let signal_json = serde_json::to_string(&signal).expect("serialize implicit signal");
        let de_signal: ImplicitSignal =
            serde_json::from_str(&signal_json).expect("deserialize signal");
        assert_eq!(signal.id, de_signal.id);
        assert_eq!(signal.kind, SignalKind::ToolLoop);
        assert_eq!(de_signal.tool_name.as_deref(), Some("file_search"));

        let mem_id = Uuid::new_v4();
        let fb_event = FeedbackEvent::new(FeedbackRating::Negative, "user_prompt")
            .with_memory_id(mem_id)
            .with_signal_id(signal.id)
            .with_comment("Outdated rule");
        let fb_json = serde_json::to_string(&fb_event).expect("serialize feedback event");
        let de_fb: FeedbackEvent =
            serde_json::from_str(&fb_json).expect("deserialize feedback event");
        assert_eq!(de_fb.rating, FeedbackRating::Negative);
        assert_eq!(de_fb.memory_id, Some(mem_id));
        assert_eq!(de_fb.signal_id, Some(signal.id));

        let pref = PreferencePair::new(
            "Write a simple function",
            "fn hello() -> &'static str { \"hello\" }",
            "fn hello() -> String { let s = String::new(); s + \"hello\" }",
            "sess-test-3",
        );
        let pref_json = serde_json::to_string(&pref).expect("serialize pref pair");
        let de_pref: PreferencePair =
            serde_json::from_str(&pref_json).expect("deserialize pref pair");
        assert_eq!(pref.id, de_pref.id);
        assert_eq!(pref.chosen, de_pref.chosen);

        let kto = KtoSample::new(
            "Solve the bug",
            "Fixed using memory bounds",
            true,
            "sess-test-3",
        );
        let kto_json = serde_json::to_string(&kto).expect("serialize kto");
        let de_kto: KtoSample = serde_json::from_str(&kto_json).expect("deserialize kto");
        assert_eq!(kto.label, de_kto.label);
        assert_eq!(kto.completion, de_kto.completion);

        let sft = SftSample::new(
            "Format code",
            "code: unformatted",
            "code: formatted",
            "sess-test-3",
        );
        let sft_json = serde_json::to_string(&sft).expect("serialize sft");
        let de_sft: SftSample = serde_json::from_str(&sft_json).expect("deserialize sft");
        assert_eq!(sft.instruction, de_sft.instruction);

        let budget_cfg = ContextBudgetConfig::new(4096, 20)
            .with_failure_patterns(true)
            .with_success_trajectories(false);
        assert_eq!(budget_cfg.max_tokens, 4096);
        assert_eq!(budget_cfg.top_k_memories, 20);
        assert!(!budget_cfg.include_success_trajectories);

        let hosts = HostTargetConfig::all();
        assert!(hosts.cursor);
        assert!(hosts.claude);
        assert!(hosts.codex);
        assert!(hosts.gemini);

        let none_hosts = HostTargetConfig::none();
        assert!(!none_hosts.cursor);

        assert_eq!(ExportFormat::Dpo.to_string(), "dpo");
        assert_eq!("kto".parse::<ExportFormat>().unwrap(), ExportFormat::Kto);
    }

    #[test]
    fn test_episode_record_and_replay_serialization() {
        let step = EpisodeStep::new(0, "run_tests")
            .with_tool("cargo_test")
            .with_input(serde_json::json!({"package": "strata-core"}))
            .with_output(serde_json::json!({"passed": 5}))
            .with_error("none");

        let ep = EpisodeRecord::new("sess-100", "agent-claude", "Ran test suite and verified")
            .with_task_id("task-123")
            .with_salience(0.85)
            .with_step(step)
            .with_outcome("all tests passing")
            .with_provenance("prompt-v1");

        let json = serde_json::to_string(&ep).expect("serialize episode");
        let de: EpisodeRecord = serde_json::from_str(&json).expect("deserialize episode");
        assert_eq!(ep.id, de.id);
        assert_eq!(ep.session_id, de.session_id);
        assert_eq!(ep.event_trace.len(), 1);
        assert_eq!(ep.salience_score, 0.85);

        let replay = EpisodeReplay {
            episode_id: ep.id,
            session_id: ep.session_id.clone(),
            agent_id: ep.agent_id.clone(),
            total_steps: 1,
            successful_steps: 1,
            failed_steps: 0,
            simulated_reward: 0.9,
            extracted_insights: vec!["Fast test execution".to_string()],
            candidate_facts: vec!["All crates compile".to_string()],
            candidate_anti_patterns: vec![],
        };
        let replay_json = serde_json::to_string(&replay).expect("serialize replay");
        let de_replay: EpisodeReplay =
            serde_json::from_str(&replay_json).expect("deserialize replay");
        assert_eq!(replay.episode_id, de_replay.episode_id);
        assert_eq!(de_replay.simulated_reward, 0.9);
    }

    #[test]
    fn test_hierarchical_episode_record() {
        // 1. Default leaf episode
        let leaf1 = EpisodeRecord::new("sess-h1", "agent-worker", "Executed step 1");
        assert_eq!(leaf1.tier, 1);
        assert!(leaf1.is_leaf());
        assert!(!leaf1.is_rollup());
        assert_eq!(leaf1.parent_id, None);
        assert!(leaf1.child_ids.is_empty());

        let leaf2 = EpisodeRecord::new("sess-h1", "agent-worker", "Executed step 2");

        // 2. Milestone Rollup (Tier 2)
        let rollup = EpisodeRecord::new("sess-h1", "agent-worker", "Completed milestone 1")
            .with_tier(2)
            .with_child_ids(vec![leaf1.id, leaf2.id]);
        assert_eq!(rollup.tier, 2);
        assert!(rollup.is_rollup());
        assert!(!rollup.is_leaf());
        assert_eq!(rollup.child_ids.len(), 2);
        assert_eq!(rollup.child_ids[0], leaf1.id);
        assert_eq!(rollup.child_ids[1], leaf2.id);

        // 3. Epoch Rollup (Tier 3)
        let epoch_id = Uuid::new_v4();
        let rollup_with_parent = rollup.clone().with_parent_id(epoch_id);
        assert_eq!(rollup_with_parent.parent_id, Some(epoch_id));

        let epoch = EpisodeRecord::new("sess-h1", "agent-worker", "Completed epoch 1")
            .with_tier(3)
            .with_child_id(rollup.id);
        assert_eq!(epoch.tier, 3);
        assert!(epoch.is_rollup());
        assert_eq!(epoch.child_ids, vec![rollup.id]);

        // 4. Serde roundtrip for rollup
        let rollup_json = serde_json::to_string(&rollup_with_parent).expect("serialize rollup");
        let de_rollup: EpisodeRecord =
            serde_json::from_str(&rollup_json).expect("deserialize rollup");
        assert_eq!(de_rollup.tier, 2);
        assert_eq!(de_rollup.parent_id, Some(epoch_id));
        assert_eq!(de_rollup.child_ids, vec![leaf1.id, leaf2.id]);

        // 5. Backwards compatibility: JSON without tier, parent_id, child_ids
        let legacy_json = serde_json::json!({
            "id": Uuid::new_v4(),
            "session_id": "sess-legacy",
            "agent_id": "agent-007",
            "task_id": null,
            "summary": "Legacy episode",
            "event_trace": [],
            "outcomes": [],
            "salience_score": 0.5,
            "state_snapshot": null,
            "causal_provenance": null,
            "consolidated": false,
            "created_at": 1700000000
        })
        .to_string();

        let de_legacy: EpisodeRecord =
            serde_json::from_str(&legacy_json).expect("deserialize legacy episode");
        assert_eq!(de_legacy.tier, 1);
        assert!(de_legacy.is_leaf());
        assert_eq!(de_legacy.parent_id, None);
        assert_eq!(de_legacy.child_ids, Vec::<Uuid>::new());
    }
}
