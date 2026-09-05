pub mod a2a;
pub mod errors;
pub mod events;
pub mod schemas;
pub mod state;
pub mod traits;

// Re-exports for convenience
pub use a2a::{AgentPresence, LeaseAcquireResult, ResourceLease};
pub use errors::StrataError;
pub use events::{
    CanonicalEvent, DataClassification, ErrorObserved, Event, EventId, EventPayload,
    MemoryConsolidated, MemoryWritten, ObservationReceived, Provenance, RetentionPolicy,
    SessionEnded, SessionStarted, TaskCompleted, TaskStarted, ToolInvoked, ToolResultReceived,
};
pub use schemas::{
    CodeAnchor, ContextBudgetConfig, DecayConfig, DecayMetrics, EpisodicMemory, EvidenceRef,
    ExportFormat, FactDependency, FactStatus, FeedbackEvent, FeedbackRating, HostTargetConfig,
    ImplicitSignal, JtmsAuditRow, KtoSample, MemoryFeedback, ParameterDef, PreferencePair,
    ProceduralExample, ProceduralSkill, ProceduralStep, SemanticFact, SftSample, SignalKind,
    SignalScores, SymbolType, SyncConfig, SyncDelta, SyncReport,
};
pub use state::{
    DigestOutput, FailurePattern, FailureSeverity, MemoryHandle, MemoryRecord, MemoryTier,
    MemoryType, OutboxEntry, OutboxStatus, Scope, SessionState, SessionStatus,
};

pub use traits::{EventStore, MemoryEngine, ReasoningEngine, Tool, ToolGateway};

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
        assert_eq!(config.max_retries, 5);
        assert_eq!(config.base_backoff_ms, 500);

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
}
