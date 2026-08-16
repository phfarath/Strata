pub mod errors;
pub mod events;
pub mod state;
pub mod traits;

// Re-exports for convenience
pub use errors::StrataError;
pub use events::{
    CanonicalEvent, DataClassification, ErrorObserved, Event, EventId, EventPayload,
    MemoryConsolidated, MemoryWritten, ObservationReceived, Provenance, RetentionPolicy,
    SessionEnded, SessionStarted, TaskCompleted, TaskStarted, ToolInvoked, ToolResultReceived,
};
pub use state::{
    DigestOutput, FailurePattern, FailureSeverity, MemoryHandle, MemoryRecord, MemoryType, Scope,
    SessionState, SessionStatus,
};
pub use traits::{EventStore, MemoryEngine, ReasoningEngine, Tool, ToolGateway};

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

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
}
