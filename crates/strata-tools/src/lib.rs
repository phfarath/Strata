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
            scope: Option<&Scope>,
            limit: usize,
        ) -> Result<Vec<MemoryRecord>, StrataError> {
            self.search_as_of(query, scope, None, None, limit).await
        }

        async fn search_as_of(
            &self,
            query: &str,
            _scope: Option<&Scope>,
            as_of_time: Option<chrono::DateTime<chrono::Utc>>,
            as_of_commit: Option<&str>,
            _limit: usize,
        ) -> Result<Vec<MemoryRecord>, StrataError> {
            let recs = self.records.lock().await;
            let filtered: Vec<MemoryRecord> = recs
                .iter()
                .filter(|r| {
                    if !r.content.contains(query)
                        && !r.summary.as_deref().unwrap_or("").contains(query)
                    {
                        return false;
                    }
                    if let Some(t) = as_of_time {
                        if !r.is_active_at(t) {
                            return false;
                        }
                    }
                    if let Some(c) = as_of_commit {
                        if !r.matches_commit(c) {
                            return false;
                        }
                    }
                    true
                })
                .cloned()
                .collect();
            Ok(filtered)
        }

        async fn get(&self, id: &Uuid) -> Result<Option<MemoryRecord>, StrataError> {
            self.get_as_of(id, None, None).await
        }

        async fn get_as_of(
            &self,
            id: &Uuid,
            as_of_time: Option<chrono::DateTime<chrono::Utc>>,
            as_of_commit: Option<&str>,
        ) -> Result<Option<MemoryRecord>, StrataError> {
            let recs = self.records.lock().await;
            let found = recs
                .iter()
                .find(|r| {
                    if r.id != *id {
                        return false;
                    }
                    if let Some(t) = as_of_time {
                        if !r.is_active_at(t) {
                            return false;
                        }
                    }
                    if let Some(c) = as_of_commit {
                        if !r.matches_commit(c) {
                            return false;
                        }
                    }
                    true
                })
                .cloned();
            Ok(found)
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
            let mut output =
                DigestOutput::new(session_id, format!("Total memories: {}", recs.len()));
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

        async fn get_symbol_anti_patterns(
            &self,
            file: &str,
            symbol: Option<&str>,
            _limit: usize,
        ) -> Result<Vec<FailurePattern>, StrataError> {
            let fails = self.failures.lock().await;
            let norm_file = file.replace('\\', "/");
            let norm_file = norm_file.trim_start_matches("./");
            let filtered: Vec<FailurePattern> = fails
                .iter()
                .filter(|f| {
                    if let Some(fp) = f.file_path() {
                        let fp_norm = fp.replace('\\', "/");
                        let fp_norm = fp_norm.trim_start_matches("./");
                        if fp_norm == norm_file || norm_file.contains(fp_norm) || fp_norm.contains(norm_file) {
                            if let Some(s) = symbol {
                                return f.symbol_name() == Some(s) || f.symbol_name().is_none();
                            }
                            return true;
                        }
                    }
                    false
                })
                .cloned()
                .collect();
            Ok(filtered)
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

        async fn execute(
            &self,
            params: serde_json::Value,
        ) -> Result<serde_json::Value, StrataError> {
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

        async fn execute(
            &self,
            _params: serde_json::Value,
        ) -> Result<serde_json::Value, StrataError> {
            Err(StrataError::ToolError(
                "Simulated database connection error".to_string(),
            ))
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

        assert!(gateway
            .invoke_with_session("echo", json!({}), None)
            .await
            .is_ok());
        assert!(gateway
            .invoke_with_session("echo", json!({}), None)
            .await
            .is_ok());

        let err = gateway
            .invoke_with_session("echo", json!({}), None)
            .await
            .unwrap_err();
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
            .invoke_with_session(
                "failing_tool",
                json!({ "target": "prod_db" }),
                Some("sess_99"),
            )
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
    async fn test_bitemporal_builtin_memory_tools() {
        let mem = Arc::new(MockMemoryEngine::new());
        let search_tool = MemorySearchTool::new(mem.clone());
        let get_tool = MemoryGetTool::new(mem.clone());

        let t0 = chrono::Utc::now() - chrono::Duration::days(30);
        let t1 = chrono::Utc::now() - chrono::Duration::days(15);
        let t_past = chrono::Utc::now() - chrono::Duration::days(20);
        let t_now = chrono::Utc::now();

        // 1. Record 1: Old architecture rule valid from t0 to t1, commit 'c1'
        let mut rec1 = MemoryRecord::new(
            strata_core::state::MemoryType::Semantic,
            "Use Redis cache for session storage.",
            Scope::Global,
        )
        .with_summary("session_storage_v1");
        rec1.created_at = t0;
        rec1.updated_at = t1;
        rec1.metadata["valid_from"] = json!(t0.to_rfc3339());
        rec1.metadata["valid_until"] = json!(t1.to_rfc3339());
        rec1.metadata["git_commit_hash"] = json!("c1");
        let id1_str = rec1.id.to_string();
        mem.records.lock().await.push(rec1);

        // 2. Record 2: New architecture rule valid from t1 onwards, commit 'c2'
        let mut rec2 = MemoryRecord::new(
            strata_core::state::MemoryType::Semantic,
            "Use JWT tokens with Redis blacklist for session storage.",
            Scope::Global,
        )
        .with_summary("session_storage_v2");
        rec2.created_at = t1;
        rec2.updated_at = t1;
        rec2.metadata["valid_from"] = json!(t1.to_rfc3339());
        rec2.metadata["git_commit_hash"] = json!("c2");
        let id2_str = rec2.id.to_string();
        mem.records.lock().await.push(rec2);

        // Test 1: Search at t_past (should only return rec1)
        let search_past = search_tool
            .execute(json!({
                "query": "session storage",
                "as_of_time": t_past.to_rfc3339()
            }))
            .await
            .unwrap();
        let past_arr = search_past.as_array().unwrap();
        assert_eq!(past_arr.len(), 1);
        assert_eq!(past_arr[0]["id"], id1_str);

        // Test 2: Search at t_now (should only return rec2)
        let search_now = search_tool
            .execute(json!({
                "query": "session storage",
                "as_of_time": t_now.to_rfc3339()
            }))
            .await
            .unwrap();
        let now_arr = search_now.as_array().unwrap();
        assert_eq!(now_arr.len(), 1);
        assert_eq!(now_arr[0]["id"], id2_str);

        // Test 3: Search by commit
        let search_c1 = search_tool
            .execute(json!({
                "query": "session storage",
                "as_of_commit": "c1"
            }))
            .await
            .unwrap();
        let c1_arr = search_c1.as_array().unwrap();
        assert_eq!(c1_arr.len(), 1);
        assert_eq!(c1_arr[0]["id"], id1_str);

        // Test 4: Get with as_of_time
        let get_past = get_tool
            .execute(json!({
                "id": id1_str,
                "as_of_time": t_past.to_rfc3339()
            }))
            .await
            .unwrap();
        assert_eq!(get_past["summary"], "session_storage_v1");

        let get_now_err = get_tool
            .execute(json!({
                "id": id1_str,
                "as_of_time": t_now.to_rfc3339()
            }))
            .await;
        assert!(get_now_err.is_err(), "rec1 should be not found at present time");
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
        assert!(decompose_res["ascii_tree"]
            .as_str()
            .unwrap()
            .contains("WAVE 0"));

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
        assert!(res["script_path"]
            .as_str()
            .unwrap()
            .contains("train_lora.py"));
        assert!(res["modelfile_path"]
            .as_str()
            .unwrap()
            .contains("Modelfile"));

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_antipattern_parser_rust_compiler_and_test_errors() {
        // 1. Cargo package ID mismatch
        let cargo_pkg_err =
            "error: package ID specification 'strata-wrong-package' did not match any packages";
        let fp1 = AntiPatternParser::parse(
            "cargo test -p strata-wrong-package",
            "",
            cargo_pkg_err,
            101,
            None,
            None,
        )
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
        let struct_err =
            "error[E0063]: missing field `code_anchor` in initializer of `SemanticFact`";
        let fp3 = AntiPatternParser::parse("cargo check", "", struct_err, 1, None, None)
            .expect("parse missing struct field");
        assert_eq!(fp3.signature, "rust_missing_struct_field");
        assert!(fp3
            .mitigation
            .contains("Initialize all required struct fields"));

        // 4. Undeclared symbol
        let sym_err = "error[E0433]: cannot find type `CodeAnchorEngine` in this scope\nuse of undeclared type `CodeAnchorEngine`";
        let fp4 = AntiPatternParser::parse("cargo check", "", sym_err, 1, None, None)
            .expect("parse undeclared symbol");
        assert_eq!(fp4.signature, "rust_undeclared_symbol");

        // 5. Cargo test assertion failure
        let test_fail = "running 1 test\ntest test_decay ... FAILED\npanicked at 'assertion `left == right` failed', src/decay.rs:42:5";
        let fp5 = AntiPatternParser::parse(
            "cargo test --package strata-memory",
            test_fail,
            "",
            101,
            None,
            None,
        )
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
        let tsc_err =
            "src/index.ts(12,5): error TS2322: Type 'string' is not assignable to type 'number'.";
        let fp2 = AntiPatternParser::parse("tsc --noEmit", "", tsc_err, 2, None, None)
            .expect("parse tsc error");
        assert_eq!(fp2.signature, "tsc_type_error");

        // 3. Python ModuleNotFoundError
        let py_err = "Traceback (most recent call last):\n  File \"app.py\", line 1\nModuleNotFoundError: No module named 'fastembed'";
        let fp3 = AntiPatternParser::parse("pytest tests/", "", py_err, 1, None, None)
            .expect("parse python module error");
        assert_eq!(fp3.signature, "python_module_not_found");

        // 4. Pytest failure
        let pytest_err =
            "FAILED tests/test_memory.py::test_decay - AssertionError: assert 0.42 == 0.50";
        let fp4 =
            AntiPatternParser::parse("pytest tests/test_memory.py", "", pytest_err, 1, None, None)
                .expect("parse pytest assertion failure");
        assert_eq!(fp4.signature, "pytest_failure");

        // 5. Port collision
        let port_err = "Error: listen EADDRINUSE: address already in use :::8080";
        let fp5 =
            AntiPatternParser::parse("cargo run --bin strata-server", "", port_err, 1, None, None)
                .expect("parse port collision");
        assert_eq!(fp5.signature, "network_port_collision");
        assert!(fp5.mitigation.contains("DO NOT hardcode static PORT"));
    }

    #[tokio::test]
    async fn test_command_interceptor_and_out_of_band_recording() {
        let mock_engine = Arc::new(MockMemoryEngine::new());
        let interceptor =
            CommandInterceptor::with_engine(Arc::clone(&mock_engine) as Arc<dyn MemoryEngine>);

        // Intercept a failed command (e.g. invalid cargo flag)
        let cmd = vec![
            "cargo".to_string(),
            "test".to_string(),
            "--package".to_string(),
            "nonexistent-crate-strata-xyz".to_string(),
        ];

        let result = interceptor
            .execute_and_intercept(
                &cmd,
                None,
                Some(Duration::from_secs(10)),
                Some("test_context"),
                None,
            )
            .await
            .expect("execute and intercept");

        assert_ne!(result.exit_code, Some(0));
        assert!(result.anti_pattern.is_some());
        assert!(result.surgical_guardrail.is_some());

        // Verify out-of-band failure recording in engine
        let failures = mock_engine
            .get_known_failures(None, None, 10)
            .await
            .unwrap();
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
        assert!(res["formatted_summary"]
            .as_str()
            .unwrap()
            .contains("Call Graph Analysis"));
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
        assert!(res["formatted_summary"]
            .as_str()
            .unwrap()
            .contains("Workspace Boundary Report"));
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
        assert!(res["formatted_summary"]
            .as_str()
            .unwrap()
            .contains("High-Level Architecture Map"));
    }

    #[tokio::test]
    async fn test_symbol_guard_tool_execution() {
        let mock_engine = Arc::new(MockMemoryEngine::new());
        let pattern = FailurePattern::new(
            "sig_borrow",
            "Borrow Conflict",
            "Simultaneous borrows",
            "Clone reference",
        )
        .with_symbol_anchor("src/auth.rs", Some("login"), Some("E0502"))
        .with_verified_correction("Use Arc::clone before moving into closure");

        mock_engine.record_failure(&pattern).await.unwrap();

        let tool = SymbolGuardTool::new(mock_engine);

        let res = tool
            .execute(json!({
                "file": "src/auth.rs",
                "symbol": "login"
            }))
            .await
            .expect("execute symbol guard");

        assert_eq!(res["status"], "success");
        assert_eq!(res["has_anti_patterns"], true);
        assert_eq!(res["count"], 1);
        let guardrails = res["advisory_guardrails"].as_array().unwrap();
        assert_eq!(guardrails.len(), 1);
        let guardrail_str = guardrails[0].as_str().unwrap();
        assert!(guardrail_str.contains("[KNOWN ANTI-PATTERN]: src/auth.rs::login (E0502)"));
        assert!(guardrail_str.contains("Verified Correction: Use Arc::clone"));
    }

    #[tokio::test]
    async fn test_historical_failures_replay_benchmark_and_gate_latency() {
        use strata_memory::SqliteStore;

        struct ReplayCase {
            command: &'static str,
            stdout: &'static str,
            stderr: &'static str,
            exit_code: i32,
            expected_file: Option<&'static str>,
            expected_symbol: Option<&'static str>,
            expected_error: Option<&'static str>,
        }

        let dataset = vec![
            // 1. Rust E0502 (Borrow checker mutable collision)
            ReplayCase {
                command: "cargo check",
                stdout: "",
                stderr: "error[E0502]: cannot borrow `*self` as mutable because it is also borrowed as immutable\n --> src/auth.rs:42:15\n42 | let x = self.get();",
                exit_code: 1,
                expected_file: Some("src/auth.rs"),
                expected_symbol: None,
                expected_error: Some("E0502"),
            },
            // 2. Rust E0063 (Missing struct field)
            ReplayCase {
                command: "cargo check --workspace",
                stdout: "",
                stderr: "error[E0063]: missing field `symbol` in initializer of `HookCommand`\n --> crates/strata-cli/src/commands/hook.rs:644:19",
                exit_code: 1,
                expected_file: Some("crates/strata-cli/src/commands/hook.rs"),
                expected_symbol: Some("HookCommand"),
                expected_error: Some("E0063"),
            },
            // 3. Rust E0433 (Undeclared module)
            ReplayCase {
                command: "cargo build",
                stdout: "",
                stderr: "error[E0433]: failed to resolve: use of undeclared crate or module `fastembed`\n --> crates/strata-memory/src/pyramid.rs:15:5",
                exit_code: 1,
                expected_file: Some("crates/strata-memory/src/pyramid.rs"),
                expected_symbol: None,
                expected_error: Some("E0433"),
            },
            // 4. Rust E0308 (Type mismatch)
            ReplayCase {
                command: "cargo check -p strata-reasoning",
                stdout: "",
                stderr: "error[E0308]: mismatched types\n --> crates/strata-reasoning/src/dag.rs:50:12\nexpected struct `Uuid`, found `&str`",
                exit_code: 1,
                expected_file: Some("crates/strata-reasoning/src/dag.rs"),
                expected_symbol: None,
                expected_error: Some("E0308"),
            },
            // 5. Rust E0599 (No method found)
            ReplayCase {
                command: "cargo check",
                stdout: "",
                stderr: "error[E0599]: no method named `find_symbol` found for struct `SqliteStore` in the current scope\n --> crates/strata-memory/src/store.rs:88:10",
                exit_code: 1,
                expected_file: Some("crates/strata-memory/src/store.rs"),
                expected_symbol: Some("SqliteStore::find_symbol"),
                expected_error: Some("E0599"),
            },
            // 6. Rust E0425 (Undeclared value)
            ReplayCase {
                command: "cargo check",
                stdout: "",
                stderr: "error[E0425]: cannot find value `token` in this scope\n --> src/middleware.rs:25:9",
                exit_code: 1,
                expected_file: Some("src/middleware.rs"),
                expected_symbol: None,
                expected_error: Some("E0425"),
            },
            // 7. Rust E0382 (Borrow of moved value)
            ReplayCase {
                command: "cargo check",
                stdout: "",
                stderr: "error[E0382]: borrow of moved value: `failure`\n --> crates/strata-cli/src/commands/hook.rs:482:33",
                exit_code: 1,
                expected_file: Some("crates/strata-cli/src/commands/hook.rs"),
                expected_symbol: None,
                expected_error: Some("E0382"),
            },
            // 8. Rust E0597 (Does not live long enough)
            ReplayCase {
                command: "cargo check",
                stdout: "",
                stderr: "error[E0597]: `val` does not live long enough\n --> src/session.rs:30:5",
                exit_code: 1,
                expected_file: Some("src/session.rs"),
                expected_symbol: None,
                expected_error: Some("E0597"),
            },
            // 9. Rust E0277 (Trait bound not satisfied)
            ReplayCase {
                command: "cargo check",
                stdout: "",
                stderr: "error[E0277]: the trait bound `MyType: Send` is not satisfied\n --> src/engine.rs:12:1",
                exit_code: 1,
                expected_file: Some("src/engine.rs"),
                expected_symbol: None,
                expected_error: Some("E0277"),
            },
            // 10. Rust E0507 (Move out of shared reference)
            ReplayCase {
                command: "cargo check",
                stdout: "",
                stderr: "error[E0507]: cannot move out of `*self` which is behind a shared reference\n --> crates/strata-core/src/events.rs:77:9",
                exit_code: 1,
                expected_file: Some("crates/strata-core/src/events.rs"),
                expected_symbol: None,
                expected_error: Some("E0507"),
            },
            // 11. Rust test panic in unit test
            ReplayCase {
                command: "cargo test -p strata-memory",
                stdout: "running 1 test\ntest tests::test_decay ... FAILED\npanicked at 'assertion `left == right` failed', crates/strata-memory/src/decay.rs:42:5",
                stderr: "",
                exit_code: 101,
                expected_file: Some("crates/strata-memory/src/decay.rs"),
                expected_symbol: Some("tests::test_decay"),
                expected_error: Some("AssertionFailure"),
            },
            // 12. Rust test panic unwrap
            ReplayCase {
                command: "cargo test",
                stdout: "thread 'tests::test_preference_miner' panicked at 'called `Option::unwrap()` on a `None` value', crates/strata-memory/src/tests.rs:991:9",
                stderr: "",
                exit_code: 101,
                expected_file: Some("crates/strata-memory/src/tests.rs"),
                expected_symbol: Some("tests::test_preference_miner"),
                expected_error: Some("AssertionFailure"),
            },
            // 13. Rust test failure summary
            ReplayCase {
                command: "cargo test",
                stdout: "test test_sync_cycle ... FAILED\nfailures:\n    test_sync_cycle",
                stderr: "",
                exit_code: 101,
                expected_file: None,
                expected_symbol: Some("test_sync_cycle"),
                expected_error: None,
            },
            // 14. Rust panic assertion with line pointer
            ReplayCase {
                command: "cargo test -p strata-tools",
                stdout: "",
                stderr: "thread 'main' panicked at crates/strata-tools/src/call_graph.rs:55:13: assertion failed: total > 0",
                exit_code: 101,
                expected_file: Some("crates/strata-tools/src/call_graph.rs"),
                expected_symbol: None,
                expected_error: Some("AssertionFailure"),
            },
            // 15. Rust Cargo package not found
            ReplayCase {
                command: "cargo test --package nonexistent-package-abc",
                stdout: "",
                stderr: "error: package ID specification 'nonexistent-package-abc' did not match any packages",
                exit_code: 101,
                expected_file: None,
                expected_symbol: None,
                expected_error: Some("CargoPackageMismatch"),
            },
            // 16. Python ModuleNotFoundError
            ReplayCase {
                command: "python labs/episodic-pyramid-slm/infer.py",
                stdout: "",
                stderr: "Traceback (most recent call last):\n  File \"labs/infer.py\", line 1, in <module>\nModuleNotFoundError: No module named 'torch'",
                exit_code: 1,
                expected_file: Some("labs/infer.py"),
                expected_symbol: None,
                expected_error: Some("ModuleNotFoundError"),
            },
            // 17. Python AttributeError
            ReplayCase {
                command: "python scripts/db_migration.py",
                stdout: "",
                stderr: "Traceback (most recent call last):\n  File \"scripts/db_migration.py\", line 42, in connect_pool\nAttributeError: 'NoneType' object has no attribute 'execute'",
                exit_code: 1,
                expected_file: Some("scripts/db_migration.py"),
                expected_symbol: Some("connect_pool"),
                expected_error: Some("AttributeError"),
            },
            // 18. Python TypeError
            ReplayCase {
                command: "python scripts/metrics.py",
                stdout: "",
                stderr: "Traceback (most recent call last):\n  File \"scripts/metrics.py\", line 15, in calculate_retention\nTypeError: unsupported operand type(s) for +: 'int' and 'str'",
                exit_code: 1,
                expected_file: Some("scripts/metrics.py"),
                expected_symbol: Some("calculate_retention"),
                expected_error: Some("TypeError"),
            },
            // 19. Python KeyError
            ReplayCase {
                command: "python labs/runner.py",
                stdout: "",
                stderr: "Traceback (most recent call last):\n  File \"labs/runner.py\", line 68, in parse_payload\nKeyError: 'session_id'",
                exit_code: 1,
                expected_file: Some("labs/runner.py"),
                expected_symbol: Some("parse_payload"),
                expected_error: Some("KeyError"),
            },
            // 20. Python IndexError
            ReplayCase {
                command: "python scripts/benchmark.py",
                stdout: "",
                stderr: "Traceback (most recent call last):\n  File \"scripts/benchmark.py\", line 33, in extract_top_p\nIndexError: list index out of range",
                exit_code: 1,
                expected_file: Some("scripts/benchmark.py"),
                expected_symbol: Some("extract_top_p"),
                expected_error: Some("IndexError"),
            },
            // 21. Python ValueError
            ReplayCase {
                command: "python labs/config.py",
                stdout: "",
                stderr: "Traceback (most recent call last):\n  File \"labs/config.py\", line 12, in load_port\nValueError: invalid literal for int() with base 10: 'abc'",
                exit_code: 1,
                expected_file: Some("labs/config.py"),
                expected_symbol: Some("load_port"),
                expected_error: Some("ValueError"),
            },
            // 22. Python FileNotFoundError
            ReplayCase {
                command: "python labs/loader.py",
                stdout: "",
                stderr: "Traceback (most recent call last):\n  File \"labs/loader.py\", line 20, in load_weights\nFileNotFoundError: [Errno 2] No such file or directory: 'model.bin'",
                exit_code: 1,
                expected_file: Some("labs/loader.py"),
                expected_symbol: Some("load_weights"),
                expected_error: Some("FileNotFoundError"),
            },
            // 23. Python ZeroDivisionError
            ReplayCase {
                command: "python scripts/eval.py",
                stdout: "",
                stderr: "Traceback (most recent call last):\n  File \"scripts/eval.py\", line 90, in compute_f1\nZeroDivisionError: division by zero",
                exit_code: 1,
                expected_file: Some("scripts/eval.py"),
                expected_symbol: Some("compute_f1"),
                expected_error: Some("ZeroDivisionError"),
            },
            // 24. Pytest assertion error
            ReplayCase {
                command: "pytest tests/test_auth.py",
                stdout: "FAILED tests/test_auth.py::test_login_flow - AssertionError: assert 401 == 200",
                stderr: "",
                exit_code: 1,
                expected_file: Some("tests/test_auth.py"),
                expected_symbol: Some("test_login_flow"),
                expected_error: Some("AssertionError"),
            },
            // 25. Pytest runtime error
            ReplayCase {
                command: "pytest tests/test_pyramid.py",
                stdout: "FAILED tests/test_pyramid.py::test_slm_bridge - RuntimeError: CUDA out of memory",
                stderr: "",
                exit_code: 1,
                expected_file: Some("tests/test_pyramid.py"),
                expected_symbol: Some("test_slm_bridge"),
                expected_error: Some("RuntimeError"),
            },
            // 26. TypeScript TS2322
            ReplayCase {
                command: "tsc --noEmit",
                stdout: "src/index.ts(12,5): error TS2322: Type 'string' is not assignable to type 'number'.",
                stderr: "",
                exit_code: 2,
                expected_file: Some("src/index.ts"),
                expected_symbol: None,
                expected_error: Some("TS2322"),
            },
            // 27. TypeScript TS2345
            ReplayCase {
                command: "tsc --noEmit",
                stdout: "src/api.ts:25:7 - error TS2345: Argument of type 'number' is not assignable to parameter of type 'string'.",
                stderr: "",
                exit_code: 2,
                expected_file: Some("src/api.ts"),
                expected_symbol: None,
                expected_error: Some("TS2345"),
            },
            // 28. TypeScript TS2554
            ReplayCase {
                command: "tsc --noEmit",
                stdout: "src/client.ts(40,10): error TS2554: Expected 2 arguments, but got 1.",
                stderr: "",
                exit_code: 2,
                expected_file: Some("src/client.ts"),
                expected_symbol: None,
                expected_error: Some("TS2554"),
            },
            // 29. Node MODULE_NOT_FOUND
            ReplayCase {
                command: "node packages/core/src/index.ts",
                stdout: "",
                stderr: "Error: Cannot find module '@strata/memory-core'\n    at Module._resolveFilename (node:internal/modules/cjs/loader:1077:15)",
                exit_code: 1,
                expected_file: None,
                expected_symbol: None,
                expected_error: Some("MODULE_NOT_FOUND"),
            },
            // 30. Node TypeError stack trace
            ReplayCase {
                command: "npm test",
                stdout: "",
                stderr: "TypeError: Cannot read properties of undefined (reading 'token')\n    at Object.processPayload (src/worker.ts:42:15)",
                exit_code: 1,
                expected_file: Some("src/worker.ts"),
                expected_symbol: Some("processPayload"),
                expected_error: Some("TypeError"),
            },
            // 31. Jest test failure
            ReplayCase {
                command: "jest tests",
                stdout: "FAIL src/auth.test.ts\n  ● auth > should fail on bad password\n    expect(received).toBe(expected)",
                stderr: "",
                exit_code: 1,
                expected_file: Some("src/auth.test.ts"),
                expected_symbol: None,
                expected_error: None,
            },
            // 32. Network EADDRINUSE port collision
            ReplayCase {
                command: "cargo run --bin server",
                stdout: "",
                stderr: "Error: listen EADDRINUSE: address already in use :::8080",
                exit_code: 1,
                expected_file: None,
                expected_symbol: None,
                expected_error: Some("EADDRINUSE"),
            },
        ];

        assert!(dataset.len() >= 30, "Replay benchmark requires at least 30 real historical failures, got {}", dataset.len());

        let store = Arc::new(SqliteStore::open_in_memory().expect("open memory store"));
        let mut correct_precision_count = 0;
        let mut parsed_patterns = Vec::new();

        for case in &dataset {
            let parsed = AntiPatternParser::parse(
                case.command,
                case.stdout,
                case.stderr,
                case.exit_code,
                None,
                None,
            );
            assert!(parsed.is_some(), "Parser failed to produce FailurePattern for: {}", case.command);
            let p = parsed.unwrap();

            // 1. Verify Precision of symbol/file/error extraction
            let mut matched = false;
            if let Some(expected_f) = case.expected_file {
                if let Some(actual_f) = p.file_path() {
                    if actual_f.contains(expected_f) || expected_f.contains(actual_f) {
                        matched = true;
                    }
                }
            }
            if let Some(expected_s) = case.expected_symbol {
                if let Some(actual_s) = p.symbol_name() {
                    if actual_s.contains(expected_s) || expected_s.contains(actual_s) {
                        matched = true;
                    }
                }
            }
            if let Some(expected_e) = case.expected_error {
                let actual_e = p.error_class().unwrap_or(&p.error_type);
                if actual_e.contains(expected_e) {
                    matched = true;
                }
            }
            if matched {
                correct_precision_count += 1;
            }

            // 2. Verify Dual-Polarity (every pattern has either verified correction or structured mitigation)
            assert!(
                p.verified_correction().is_some() || !p.mitigation.is_empty(),
                "Pattern must have verified_correction or mitigation"
            );

            // 3. Verify Surgical Guardrail token length (< 50 tokens)
            let guardrail = AntiPatternParser::format_surgical_guardrail(&p);
            let word_count = guardrail.split_whitespace().count();
            // 1 word is approx 1.3 tokens; 40 words is <= 50 tokens
            let est_tokens = (word_count as f64 * 1.3).ceil() as usize;
            assert!(
                est_tokens < 50,
                "Surgical guardrail must be < 50 tokens, got {} tokens (words={}): '{}'",
                est_tokens, word_count, guardrail
            );

            // Seed into SQLite store for latency benchmark
            store.upsert_failure_pattern(&p).expect("upsert failure into store");
            parsed_patterns.push(p);
        }

        // Calculate precision
        let precision = (correct_precision_count as f64) / (dataset.len() as f64);
        println!(
            "Replay Benchmark Precision: {:.2}% ({}/{} failures matched target file/symbol/error)",
            precision * 100.0,
            correct_precision_count,
            dataset.len()
        );
        assert!(
            precision >= 0.80,
            "Replay precision must be >= 80%, got {:.2}%",
            precision * 100.0
        );

        // 4. Benchmark Deterministic Gate Latency (p95 < 10ms)
        let mut latencies_us = Vec::with_capacity(1000);
        for i in 0..1000 {
            let pat = &parsed_patterns[i % parsed_patterns.len()];
            let target_file = pat.file_path().unwrap_or("src/auth.rs");
            let target_symbol = pat.symbol_name();

            let start = std::time::Instant::now();
            let _ = store.find_symbol_anti_patterns(target_file, target_symbol, 3).unwrap();
            latencies_us.push(start.elapsed().as_micros());
        }

        latencies_us.sort_unstable();
        let p50 = latencies_us[(latencies_us.len() * 50) / 100];
        let p95 = latencies_us[(latencies_us.len() * 95) / 100];
        let p99 = latencies_us[(latencies_us.len() * 99) / 100];

        println!(
            "Deterministic Gate Latency across 1,000 queries: p50={}µs, p95={}µs, p99={}µs",
            p50, p95, p99
        );
        assert!(
            p95 < 10_000,
            "Gate latency p95 must be < 10ms (10,000µs), got {}µs",
            p95
        );
    }
}
