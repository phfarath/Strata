use std::sync::Arc;
use anyhow::{bail, Result};
use strata_cli::mcp::{
    protocol::{
        negotiate_protocol_version, CallToolResult, JsonRpcRequest, ToolsListResult,
        DEFAULT_PROTOCOL_VERSION, LATEST_PROTOCOL_VERSION,
    },
    server::McpServer,
};
use strata_memory::SqliteMemoryEngine;

/// Evaluation Scenario: MCP Multi-Version Protocol & Transport (2024-11-05, 2025-11-25, 2026-07-28)
pub async fn run_mcp_protocol_multi_version_scenario() -> Result<()> {
    println!("\n▶ Running Eval Scenario: MCP Multi-Version Protocol & Transport");

    // Initialize in-memory SQLite engine
    let engine = Arc::new(SqliteMemoryEngine::open_in_memory(None)?);
    let server = McpServer::new(engine.clone());

    // -------------------------------------------------------------------------
    // Test A: Handshake negotiation with 2024-11-05, 2025-11-25, 2026-07-28
    // -------------------------------------------------------------------------
    println!("  [Test A] Testing protocol version negotiation...");

    // 1. Client requests 2024-11-05
    let req_2024 = JsonRpcRequest {
        jsonrpc: "2.0".to_string(),
        id: Some(serde_json::json!(1)),
        method: "initialize".to_string(),
        params: Some(serde_json::json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": { "name": "cursor", "version": "1.0.0" }
        })),
        meta: None,
    };
    let resp_2024 = server.handle_request(req_2024).await.expect("Expected response for initialize 2024-11-05");
    let result_2024 = resp_2024.result.expect("Expected result in initialize response");
    if result_2024.get("protocolVersion").and_then(|v| v.as_str()) != Some("2024-11-05") {
        bail!("Failed negotiation for 2024-11-05: got {:?}", result_2024.get("protocolVersion"));
    }
    println!("    ✓ Handshake negotiated '2024-11-05' successfully");

    // 2. Client requests 2025-11-25
    let req_2025 = JsonRpcRequest {
        jsonrpc: "2.0".to_string(),
        id: Some(serde_json::json!(2)),
        method: "initialize".to_string(),
        params: Some(serde_json::json!({
            "protocolVersion": "2025-11-25",
            "capabilities": {},
            "clientInfo": { "name": "claude-code", "version": "2.0.0" }
        })),
        meta: None,
    };
    let resp_2025 = server.handle_request(req_2025).await.expect("Expected response for initialize 2025-11-25");
    let result_2025 = resp_2025.result.expect("Expected result in initialize response");
    if result_2025.get("protocolVersion").and_then(|v| v.as_str()) != Some("2025-11-25") {
        bail!("Failed negotiation for 2025-11-25: got {:?}", result_2025.get("protocolVersion"));
    }
    println!("    ✓ Handshake negotiated '2025-11-25' successfully");

    // 3. Client requests 2026-07-28
    let req_2026 = JsonRpcRequest {
        jsonrpc: "2.0".to_string(),
        id: Some(serde_json::json!(3)),
        method: "initialize".to_string(),
        params: Some(serde_json::json!({
            "protocolVersion": "2026-07-28",
            "capabilities": {},
            "clientInfo": { "name": "next-gen-agent", "version": "3.0.0" }
        })),
        meta: None,
    };
    let resp_2026 = server.handle_request(req_2026).await.expect("Expected response for initialize 2026-07-28");
    let result_2026 = resp_2026.result.expect("Expected result in initialize response");
    if result_2026.get("protocolVersion").and_then(|v| v.as_str()) != Some("2026-07-28") {
        bail!("Failed negotiation for 2026-07-28: got {:?}", result_2026.get("protocolVersion"));
    }
    println!("    ✓ Handshake negotiated '2026-07-28' successfully");

    // 4. Default / unknown negotiation verification
    let default_neg = negotiate_protocol_version(None);
    assert_eq!(default_neg, DEFAULT_PROTOCOL_VERSION);
    let unknown_neg = negotiate_protocol_version(Some("9999-99-99"));
    assert_eq!(unknown_neg, LATEST_PROTOCOL_VERSION);

    // -------------------------------------------------------------------------
    // Test B: tools/list returns all 5 tools with cache hints (_meta)
    // -------------------------------------------------------------------------
    println!("  [Test B] Testing tools/list with _meta cache hints...");

    let req_tools = JsonRpcRequest {
        jsonrpc: "2.0".to_string(),
        id: Some(serde_json::json!(4)),
        method: "tools/list".to_string(),
        params: None,
        meta: None,
    };
    let resp_tools = server.handle_request(req_tools).await.expect("Expected response for tools/list");
    let result_tools = resp_tools.result.expect("Expected result in tools/list response");
    let tools_list: ToolsListResult = serde_json::from_value(result_tools.clone())?;

    let tool_names: Vec<String> = tools_list.tools.iter().map(|t| t.name.clone()).collect();
    println!("    Discovered tools: {:?}", tool_names);

    if tools_list.tools.len() < 6 {
        bail!("Expected at least 6 tools registered, got {}", tools_list.tools.len());
    }

    let expected_tools = [
        "memory_search",
        "memory_get",
        "memory_write",
        "memory_digest",
        "memory_feedback",
        "causal_blast_radius",
    ];
    for exp in expected_tools {
        if !tool_names.contains(&exp.to_string()) {
            bail!("Missing expected tool: '{exp}'");
        }
    }

    // Check _meta cache hint
    let meta = tools_list.meta.expect("Expected _meta field on tools/list response");
    if meta.get("ttlMs").and_then(|v| v.as_u64()) != Some(3600000) {
        bail!("Expected _meta.ttlMs = 3600000, got {:?}", meta.get("ttlMs"));
    }
    if meta.get("cacheScope").and_then(|v| v.as_str()) != Some("session") {
        bail!("Expected _meta.cacheScope = 'session', got {:?}", meta.get("cacheScope"));
    }
    println!("    ✓ tools/list returned 5 tools and verified _meta cache hints: {:?}", meta);

    // -------------------------------------------------------------------------
    // Test C: Execution of all 5 tools (search, get, write, digest, feedback)
    // -------------------------------------------------------------------------
    println!("  [Test C] Testing execution of all 5 tools...");

    // 1. Tool: memory_write
    let write_res = server.execute_tool("memory_write", serde_json::json!({
        "content": "Strata implements deterministic multi-version MCP protocol with stateless invocation support.",
        "summary": "MCP multi-version transport design",
        "memory_type": "semantic",
        "scope": "global",
        "tags": ["mcp", "protocol", "architecture"],
        "importance": 0.9,
        "confidence": 0.85
    })).await;

    if write_res.is_error == Some(true) {
        bail!("memory_write tool execution failed: {:?}", write_res.content);
    }
    println!("    ✓ memory_write succeeded: {}", write_res.content[0].text);

    // 2. Tool: memory_search
    let search_res = server.execute_tool("memory_search", serde_json::json!({
        "query": "multi-version MCP protocol",
        "limit": 5
    })).await;

    if search_res.is_error == Some(true) {
        bail!("memory_search tool execution failed: {:?}", search_res.content);
    }
    let search_json: serde_json::Value = serde_json::from_str(&search_res.content[0].text)?;
    let search_arr = search_json.as_array().expect("Search results should be JSON array");
    if search_arr.is_empty() {
        bail!("memory_search returned 0 results for written memory");
    }
    let memory_id = search_arr[0].get("id").and_then(|v| v.as_str()).expect("Memory ID in handle");
    println!("    ✓ memory_search succeeded, found memory ID: {}", memory_id);

    // 3. Tool: memory_get
    let get_res = server.execute_tool("memory_get", serde_json::json!({
        "id": memory_id
    })).await;

    if get_res.is_error == Some(true) {
        bail!("memory_get tool execution failed: {:?}", get_res.content);
    }
    let get_json: serde_json::Value = serde_json::from_str(&get_res.content[0].text)?;
    if get_json.get("id").and_then(|v| v.as_str()) != Some(memory_id) {
        bail!("memory_get ID mismatch: expected {}, got {:?}", memory_id, get_json.get("id"));
    }
    println!("    ✓ memory_get succeeded: summary='{}'", get_json.get("summary").and_then(|v| v.as_str()).unwrap_or(""));

    // 4. Tool: memory_feedback
    let feedback_res = server.execute_tool("memory_feedback", serde_json::json!({
        "id": memory_id,
        "rating": "positive",
        "score": 0.98,
        "comment": "Accurate retrieval and solid multi-version architecture"
    })).await;

    if feedback_res.is_error == Some(true) {
        bail!("memory_feedback tool execution failed: {:?}", feedback_res.content);
    }
    let fb_data = feedback_res.structured_content.expect("memory_feedback should return structured_content");
    if fb_data.get("status").and_then(|v| v.as_str()) != Some("feedback_recorded") {
        bail!("Expected feedback_recorded status, got {:?}", fb_data.get("status"));
    }
    let conf = fb_data.get("confidence").and_then(|v| v.as_f64()).unwrap_or(0.0);
    if (conf - 0.98).abs() > 0.001 {
        bail!("Expected updated confidence ~0.98, got {:?}", fb_data.get("confidence"));
    }
    println!("    ✓ memory_feedback succeeded: updated confidence to {}", fb_data["confidence"]);


    // 5. Tool: memory_digest
    let digest_res = server.execute_tool("memory_digest", serde_json::json!({
        "session_id": "eval-session-mcp",
        "max_tokens": 400
    })).await;

    if digest_res.is_error == Some(true) {
        bail!("memory_digest tool execution failed: {:?}", digest_res.content);
    }
    println!("    ✓ memory_digest succeeded");

    // -------------------------------------------------------------------------
    // Test D: 2026 Stateless execution (calling tools/call directly without prior initialize)
    // -------------------------------------------------------------------------
    println!("  [Test D] Testing 2026 stateless invocation (tools/call without initialize)...");

    let stateless_server = McpServer::new(engine.clone());
    let direct_call_req = JsonRpcRequest {
        jsonrpc: "2.0".to_string(),
        id: Some(serde_json::json!("stateless-req-101")),
        method: "tools/call".to_string(),
        params: Some(serde_json::json!({
            "name": "memory_search",
            "arguments": {
                "query": "stateless invocation",
                "limit": 3
            }
        })),
        meta: None,
    };

    let direct_call_resp = stateless_server.handle_request(direct_call_req).await
        .expect("Stateless server MUST handle tools/call directly without prior initialize");

    if direct_call_resp.error.is_some() {
        bail!("Stateless tools/call returned error: {:?}", direct_call_resp.error);
    }
    let direct_result = direct_call_resp.result.expect("Expected result from stateless tools/call");
    let call_res: CallToolResult = serde_json::from_value(direct_result)?;
    if call_res.is_error == Some(true) {
        bail!("Direct tool execution returned tool error: {:?}", call_res.content);
    }
    println!("    ✓ Stateless tools/call succeeded cleanly without handshake");

    println!("  ✓ MCP Protocol Multi-Version evaluation scenario PASSED (4/4 tests).\n");
    Ok(())
}
