pub mod builtin;
pub mod gateway;

pub use builtin::*;
pub use gateway::*;

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use serde_json::json;
    use std::sync::Arc;
    use tokio::sync::Mutex;
    use uuid::Uuid;

    use strata_core::{
        errors::StrataError,
        events::{Event, EventId, EventPayload},
        state::{DigestOutput, FailurePattern, MemoryHandle, MemoryRecord, Scope},
        traits::{EventStore, MemoryEngine, Tool},
    };

    // --- Mock Memory Engine for Testing ---
    struct MockMemoryEngine {
        records: Arc<Mutex<Vec<MemoryRecord>>>,
        failures: Arc<Mutex<Vec<FailurePattern>>>,
    }

    impl MockMemoryEngine {
        fn new() -> Self {
            Self {
                records: Arc::new(Mutex::new(Vec::new())),
                failures: Arc::new(Mutex::new(Vec::new())),
            }
        }
    }

    #[async_trait]
    impl MemoryEngine for MockMemoryEngine {
        async fn search(
            &self,
            query: &str,
            _scope: Option<&Scope>,
            _limit: usize,
        ) -> Result<Vec<MemoryRecord>, StrataError> {
            let recs = self.records.lock().await;
            let filtered: Vec<MemoryRecord> = recs
                .iter()
                .filter(|r| r.content.contains(query) || r.summary.as_deref().unwrap_or("").contains(query))
                .cloned()
                .collect();
            Ok(filtered)
        }

        async fn get(&self, id: &Uuid) -> Result<Option<MemoryRecord>, StrataError> {
            let recs = self.records.lock().await;
            Ok(recs.iter().find(|r| r.id == *id).cloned())
        }

        async fn write(&self, record: &MemoryRecord) -> Result<MemoryHandle, StrataError> {
            let mut recs = self.records.lock().await;
            recs.push(record.clone());
            Ok(record.to_handle(Some(1.0)))
        }

        async fn digest(
            &self,
            session_id: &str,
            _max_tokens: Option<usize>,
        ) -> Result<DigestOutput, StrataError> {
            let recs = self.records.lock().await;
            let fails = self.failures.lock().await;
            let mut output = DigestOutput::new(session_id, format!("Total memories: {}", recs.len()));
            output.recent_decisions = vec!["Adopted Rust workspace".to_string()];
            output.failure_warnings = fails.clone();
            Ok(output)
        }

        async fn record_failure(&self, failure: &FailurePattern) -> Result<(), StrataError> {
            let mut fails = self.failures.lock().await;
            fails.push(failure.clone());
            Ok(())
        }

        async fn get_known_failures(
            &self,
            _query: Option<&str>,
            _scope: Option<&Scope>,
            _limit: usize,
        ) -> Result<Vec<FailurePattern>, StrataError> {
            let fails = self.failures.lock().await;
            Ok(fails.clone())
        }
    }

    // --- Mock Event Store for Testing ---
    struct MockEventStore {
        events: Arc<Mutex<Vec<Event>>>,
    }

    impl MockEventStore {
        fn new() -> Self {
            Self {
                events: Arc::new(Mutex::new(Vec::new())),
            }
        }
    }

    #[async_trait]
    impl EventStore for MockEventStore {
        async fn append(&self, event: &Event) -> Result<EventId, StrataError> {
            let mut evs = self.events.lock().await;
            let id = event.id;
            evs.push(event.clone());
            Ok(id)
        }

        async fn read_stream(
            &self,
            _session_id: &str,
            _from_seq: Option<u64>,
            _limit: Option<usize>,
        ) -> Result<Vec<Event>, StrataError> {
            let evs = self.events.lock().await;
            Ok(evs.clone())
        }
    }

    // --- Mock Tool for Testing ---
    struct EchoTool;

    #[async_trait]
    impl Tool for EchoTool {
        fn name(&self) -> &str {
            "echo"
        }

        fn description(&self) -> &str {
            "Echoes back input"
        }

        fn parameters_schema(&self) -> serde_json::Value {
            json!({ "type": "object", "properties": { "msg": { "type": "string" } } })
        }

        async fn execute(&self, params: serde_json::Value) -> Result<serde_json::Value, StrataError> {
            Ok(params)
        }
    }

    struct FailingTool;

    #[async_trait]
    impl Tool for FailingTool {
        fn name(&self) -> &str {
            "failing_tool"
        }

        fn description(&self) -> &str {
            "Always fails"
        }

        fn parameters_schema(&self) -> serde_json::Value {
            json!({})
        }

        async fn execute(&self, _params: serde_json::Value) -> Result<serde_json::Value, StrataError> {
            Err(StrataError::ToolError("Simulated database connection error".to_string()))
        }
    }

    #[tokio::test]
    async fn test_gateway_invocation_and_audit() {
        let mem = Arc::new(MockMemoryEngine::new());
        let store = Arc::new(MockEventStore::new());

        let gateway = DefaultToolGateway::new()
            .with_event_store(store.clone())
            .with_memory_engine(mem.clone());

        gateway.register(Arc::new(EchoTool)).await;

        let result = gateway
            .invoke_with_session("echo", json!({ "msg": "hello world" }), Some("sess_123"))
            .await
            .unwrap();

        assert_eq!(result["msg"], "hello world");

        let events = store.events.lock().await;
        assert_eq!(events.len(), 2); // ToolInvoked and ToolResultReceived
        match &events[0].payload {
            EventPayload::ToolInvoked(t) => assert_eq!(t.tool_name, "echo"),
            _ => panic!("Expected ToolInvoked"),
        }
        match &events[1].payload {
            EventPayload::ToolResultReceived(t) => {
                assert_eq!(t.tool_name, "echo");
                assert!(!t.is_error);
            }
            _ => panic!("Expected ToolResultReceived"),
        }
    }

    #[tokio::test]
    async fn test_gateway_blocked_tool_permission() {
        let policy = PermissionPolicy::default().with_blocked_tool("dangerous_tool");
        let gateway = DefaultToolGateway::new().with_policy(policy);

        gateway.register(Arc::new(EchoTool)).await;

        let err = gateway
            .invoke_with_session("dangerous_tool", json!({}), None)
            .await
            .unwrap_err();

        match err {
            StrataError::PermissionDenied(msg) => {
                assert!(msg.contains("blocked by security policy"));
            }
            _ => panic!("Expected PermissionDenied"),
        }
    }

    #[tokio::test]
    async fn test_gateway_rate_limiting() {
        let policy = PermissionPolicy::default().with_rate_limit(2);
        let gateway = DefaultToolGateway::new().with_policy(policy);
        gateway.register(Arc::new(EchoTool)).await;

        assert!(gateway.invoke_with_session("echo", json!({}), None).await.is_ok());
        assert!(gateway.invoke_with_session("echo", json!({}), None).await.is_ok());

        let err = gateway.invoke_with_session("echo", json!({}), None).await.unwrap_err();
        match err {
            StrataError::RateLimitExceeded(_) => {}
            _ => panic!("Expected RateLimitExceeded"),
        }
    }

    #[tokio::test]
    async fn test_automatic_failure_capture_out_of_band() {
        let mem = Arc::new(MockMemoryEngine::new());
        let store = Arc::new(MockEventStore::new());

        let gateway = DefaultToolGateway::new()
            .with_event_store(store.clone())
            .with_memory_engine(mem.clone());

        gateway.register(Arc::new(FailingTool)).await;

        let err = gateway
            .invoke_with_session("failing_tool", json!({ "target": "prod_db" }), Some("sess_99"))
            .await
            .unwrap_err();

        match err {
            StrataError::ToolError(msg) => assert!(msg.contains("Simulated database")),
            _ => panic!("Expected ToolError"),
        }

        // Check that failure was recorded into memory silently out-of-band
        let fails = mem.failures.lock().await;
        assert_eq!(fails.len(), 1);
        assert_eq!(fails[0].pattern_name, "failing_tool Failure");
        assert_eq!(fails[0].error_type, "ToolExecutionError");
        assert!(fails[0].description.contains("Simulated database"));
    }

    #[tokio::test]
    async fn test_builtin_memory_tools() {
        let mem = Arc::new(MockMemoryEngine::new());

        let write_tool = MemoryWriteTool::new(mem.clone());
        let search_tool = MemorySearchTool::new(mem.clone());
        let digest_tool = MemoryDigestTool::new(mem.clone());

        // 1. Write memory
        let write_res = write_tool
            .execute(json!({
                "content": "Use SQLite FTS5 for hybrid search",
                "summary": "arch_decision",
                "memory_type": "semantic",
                "importance": 0.8
            }))
            .await
            .unwrap();

        let id_str = write_res["id"].as_str().unwrap();

        // 2. Search memory
        let search_res = search_tool
            .execute(json!({ "query": "SQLite FTS5" }))
            .await
            .unwrap();

        let results = search_res.as_array().unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0]["summary"], "arch_decision");

        // 3. Get memory
        let get_tool = MemoryGetTool::new(mem.clone());
        let get_res = get_tool.execute(json!({ "id": id_str })).await.unwrap();
        assert_eq!(get_res["summary"], "arch_decision");

        // 4. Digest
        let digest_res = digest_tool.execute(json!({})).await.unwrap();
        assert_eq!(digest_res["summary"], "Total memories: 1");
    }

    #[tokio::test]
    async fn test_safe_shell_blocked_command() {
        let shell = SafeShellTool::new();
        let err = shell
            .execute(json!({ "command": "rm -rf / something" }))
            .await
            .unwrap_err();

        match err {
            StrataError::PermissionDenied(msg) => {
                assert!(msg.contains("blocked pattern"));
            }
            _ => panic!("Expected PermissionDenied"),
        }
    }
}
