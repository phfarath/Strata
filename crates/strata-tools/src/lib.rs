pub mod builtin;
pub mod gateway;
pub mod interceptor;

pub use builtin::*;
pub use gateway::*;
pub use interceptor::*;

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use serde_json::json;
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::sync::Mutex;
    use uuid::Uuid;

    use strata_core::{
        errors::StrataError,
        events::{Event, EventId, EventPayload},
        state::{DigestOutput, FailurePattern, FailureSeverity, MemoryHandle, MemoryRecord, Scope},
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

    #[tokio::test]
    async fn test_goal_decompose_and_execute_tools() {
        let decompose_tool = GoalDecomposeTool::new();
        let decompose_res = decompose_tool
            .execute(json!({
                "goal": "Refactor database engine to async"
            }))
            .await
            .unwrap();

        assert_eq!(decompose_res["status"], "success");
        assert!(decompose_res["total_waves"].as_u64().unwrap() >= 3);
        assert!(decompose_res["ascii_tree"].as_str().unwrap().contains("WAVE 0"));

        let dag_val = decompose_res["dag"].clone();

        let execute_tool = DagExecuteTool::new();
        let exec_res = execute_tool
            .execute(json!({
                "dag": dag_val,
                "max_concurrency": 2
            }))
            .await
            .unwrap();

        assert_eq!(exec_res["status"], "success");
        assert!(exec_res["report"]["success"].as_bool().unwrap());
        assert!(exec_res["report"]["completed_nodes"].as_u64().unwrap() > 0);
    }

    #[tokio::test]
    async fn test_train_pipeline_tool() {
        let temp_dir = std::env::temp_dir().join("strata_tool_test_lora_run");
        let tool = TrainPipelineTool::new();

        let res = tool
            .execute(json!({
                "base_model": "unsloth/Llama-3.2-1B-Instruct",
                "method": "dpo",
                "output_dir": temp_dir.to_string_lossy(),
                "dataset_content": "{\"prompt\":\"test prompt\",\"chosen\":\"test chosen\",\"rejected\":\"test rejected\"}\n",
                "ollama_model_name": "strata-test-model",
                "dry_run": true
            }))
            .await
            .unwrap();

        assert_eq!(res["status"], "success");
        assert_eq!(res["total_samples"], 1);
        assert!(res["script_path"].as_str().unwrap().contains("train_lora.py"));
        assert!(res["modelfile_path"].as_str().unwrap().contains("Modelfile"));

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_antipattern_parser_rust_compiler_and_test_errors() {
        // 1. Cargo package ID mismatch
        let cargo_pkg_err = "error: package ID specification 'strata-wrong-package' did not match any packages";
        let fp1 = AntiPatternParser::parse("cargo test -p strata-wrong-package", "", cargo_pkg_err, 101, None, None)
            .expect("parse cargo package mismatch");
        assert_eq!(fp1.signature, "cargo_package_not_found");
        assert!(fp1.mitigation.contains("exact package name"));
        let guardrail1 = AntiPatternParser::format_surgical_guardrail(&fp1);
        assert!(guardrail1.contains("[KNOWN ANTI-PATTERN]:"));
        assert!(guardrail1.len() < 250);

        // 2. Borrow checker error
        let borrow_err = "error[E0502]: cannot borrow `data` as mutable because it is also borrowed as immutable\n  --> src/main.rs:14:5";
        let fp2 = AntiPatternParser::parse("cargo build", "", borrow_err, 1, None, None)
            .expect("parse borrow checker error");
        assert_eq!(fp2.signature, "rust_borrow_checker_conflict");
        assert_eq!(fp2.severity, FailureSeverity::High);

        // 3. Missing struct field
        let struct_err = "error[E0063]: missing field `code_anchor` in initializer of `SemanticFact`";
        let fp3 = AntiPatternParser::parse("cargo check", "", struct_err, 1, None, None)
            .expect("parse missing struct field");
        assert_eq!(fp3.signature, "rust_missing_struct_field");
        assert!(fp3.mitigation.contains("Initialize all required struct fields"));

        // 4. Undeclared symbol
        let sym_err = "error[E0433]: cannot find type `CodeAnchorEngine` in this scope\nuse of undeclared type `CodeAnchorEngine`";
        let fp4 = AntiPatternParser::parse("cargo check", "", sym_err, 1, None, None)
            .expect("parse undeclared symbol");
        assert_eq!(fp4.signature, "rust_undeclared_symbol");

        // 5. Cargo test assertion failure
        let test_fail = "running 1 test\ntest test_decay ... FAILED\npanicked at 'assertion `left == right` failed', src/decay.rs:42:5";
        let fp5 = AntiPatternParser::parse("cargo test --package strata-memory", test_fail, "", 101, None, None)
            .expect("parse cargo test failure");
        assert_eq!(fp5.signature, "cargo_test_failure");
    }

    #[test]
    fn test_antipattern_parser_npm_python_and_network_errors() {
        // 1. NPM Module Not Found
        let npm_err = "Error: Cannot find module '@strata/memory-core' or its corresponding type declarations.";
        let fp1 = AntiPatternParser::parse("npm test", "", npm_err, 1, None, None)
            .expect("parse npm module not found");
        assert_eq!(fp1.signature, "npm_module_not_found");

        // 2. TypeScript error
        let tsc_err = "src/index.ts(12,5): error TS2322: Type 'string' is not assignable to type 'number'.";
        let fp2 = AntiPatternParser::parse("tsc --noEmit", "", tsc_err, 2, None, None)
            .expect("parse tsc error");
        assert_eq!(fp2.signature, "tsc_type_error");

        // 3. Python ModuleNotFoundError
        let py_err = "Traceback (most recent call last):\n  File \"app.py\", line 1\nModuleNotFoundError: No module named 'fastembed'";
        let fp3 = AntiPatternParser::parse("pytest tests/", "", py_err, 1, None, None)
            .expect("parse python module error");
        assert_eq!(fp3.signature, "python_module_not_found");

        // 4. Pytest failure
        let pytest_err = "FAILED tests/test_memory.py::test_decay - AssertionError: assert 0.42 == 0.50";
        let fp4 = AntiPatternParser::parse("pytest tests/test_memory.py", "", pytest_err, 1, None, None)
            .expect("parse pytest assertion failure");
        assert_eq!(fp4.signature, "pytest_failure");

        // 5. Port collision
        let port_err = "Error: listen EADDRINUSE: address already in use :::8080";
        let fp5 = AntiPatternParser::parse("cargo run --bin strata-server", "", port_err, 1, None, None)
            .expect("parse port collision");
        assert_eq!(fp5.signature, "network_port_collision");
        assert!(fp5.mitigation.contains("DO NOT hardcode static PORT"));
    }

    #[tokio::test]
    async fn test_command_interceptor_and_out_of_band_recording() {
        let mock_engine = Arc::new(MockMemoryEngine::new());
        let interceptor = CommandInterceptor::with_engine(Arc::clone(&mock_engine) as Arc<dyn MemoryEngine>);

        // Intercept a failed command (e.g. invalid cargo flag)
        let cmd = vec![
            "cargo".to_string(),
            "test".to_string(),
            "--package".to_string(),
            "nonexistent-crate-strata-xyz".to_string(),
        ];

        let result = interceptor
            .execute_and_intercept(&cmd, None, Some(Duration::from_secs(10)), Some("test_context"), None)
            .await
            .expect("execute and intercept");

        assert_ne!(result.exit_code, Some(0));
        assert!(result.anti_pattern.is_some());
        assert!(result.surgical_guardrail.is_some());

        // Verify out-of-band failure recording in engine
        let failures = mock_engine.get_known_failures(None, None, 10).await.unwrap();
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0].signature, "cargo_package_not_found");
    }

    #[tokio::test]
    async fn test_call_graph_tool_execution() {
        let tool = CallGraphTool::new();

        let code = r#"
use std::collections::HashMap;

pub fn orchestrate_task(task_id: &str) -> bool {
    let ok = check_permission(task_id);
    if ok {
        execute_step();
    }
    ok
}

fn check_permission(id: &str) -> bool {
    true
}

fn execute_step() {}
"#;

        let res = tool
            .execute(json!({
                "code": code,
                "path": "src/orchestrator.rs",
                "symbol": "check_permission",
                "direction": "callers"
            }))
            .await
            .expect("execute call graph tool");

        assert_eq!(res["status"], "success");
        assert_eq!(res["total_edges"], 3);
        let callers = res["callers"].as_array().unwrap();
        assert_eq!(callers.len(), 1);
        assert_eq!(callers[0]["caller_symbol"], "orchestrate_task");
        assert!(res["formatted_summary"].as_str().unwrap().contains("Call Graph Analysis"));
    }

    #[tokio::test]
    async fn test_workspace_detect_tool_execution() {
        let tool = WorkspaceDetectTool::new();
        let current_dir = std::env::current_dir().unwrap();

        let res = tool
            .execute(json!({
                "root_path": current_dir.to_str().unwrap(),
                "file_path": "crates/strata-core/src/state.rs"
            }))
            .await
            .expect("execute workspace detect tool");

        assert_eq!(res["status"], "success");
        assert_eq!(res["workspace_type"], "cargo_workspace");
        assert!(res["packages_count"].as_u64().unwrap() >= 5);
        assert_eq!(res["resolved_package"]["name"], "strata-core");
        assert!(res["formatted_summary"].as_str().unwrap().contains("Workspace Boundary Report"));
    }

    #[tokio::test]
    async fn test_architecture_map_tool_execution() {
        let tool = ArchitectureMapTool::new();
        let current_dir = std::env::current_dir().unwrap();
        let target_dir = if current_dir.join("src").exists() {
            current_dir.join("src")
        } else {
            current_dir.clone()
        };

        let res = tool
            .execute(json!({
                "path": target_dir.to_str().unwrap(),
                "workspace_id": "test-strata-tools"
            }))
            .await
            .expect("execute architecture map tool");

        assert_eq!(res["status"], "success");
        assert_eq!(res["workspace_id"], "test-strata-tools");
        assert!(res["clusters_count"].as_u64().unwrap() >= 1);
        assert!(res["formatted_summary"].as_str().unwrap().contains("High-Level Architecture Map"));
    }
}
