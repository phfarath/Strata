use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tracing::{debug, error, info};
use uuid::Uuid;

use strata_core::{
    state::{MemoryRecord, MemoryType, Scope},
    traits::MemoryEngine,
};

use super::protocol::*;

pub struct McpServer {
    memory_engine: Arc<dyn MemoryEngine>,
    server_name: String,
    server_version: String,
}

impl McpServer {
    pub fn new(memory_engine: Arc<dyn MemoryEngine>) -> Self {
        Self {
            memory_engine,
            server_name: "strata-mcp".to_string(),
            server_version: env!("CARGO_PKG_VERSION").to_string(),
        }
    }

    pub fn tool_definitions() -> Vec<ToolDefinition> {
        vec![
            ToolDefinition {
                name: "memory_search".to_string(),
                description: "Search persistent memory records across sessions using hybrid semantic and lexical search. Returns relevant pointers, summaries, and confidence scores.".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "query": {
                            "type": "string",
                            "description": "The search query or prompt to find relevant past decisions, patterns, or context"
                        },
                        "limit": {
                            "type": "integer",
                            "description": "Maximum number of memories to return (default: 5)"
                        },
                        "scope": {
                            "type": "string",
                            "description": "Optional scope filter: 'global', 'project:<name>', 'session:<id>', 'org:<name>'"
                        },
                        "memory_type": {
                            "type": "string",
                            "enum": ["episodic", "semantic", "procedural", "negative_pattern"],
                            "description": "Optional filter by memory category"
                        }
                    },
                    "required": ["query"]
                }),
            },
            ToolDefinition {
                name: "memory_get".to_string(),
                description: "Retrieve full details and content of a specific memory record by its UUID.".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "id": {
                            "type": "string",
                            "description": "The UUID of the memory record to retrieve"
                        }
                    },
                    "required": ["id"]
                }),
            },
            ToolDefinition {
                name: "memory_write".to_string(),
                description: "Record a durable decision, architectural pattern, insight, or known failure into Strata persistent memory.".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "content": {
                            "type": "string",
                            "description": "The full content/detail of the memory to record"
                        },
                        "summary": {
                            "type": "string",
                            "description": "Short headline or mnemonic summary (< 60 chars)"
                        },
                        "memory_type": {
                            "type": "string",
                            "enum": ["episodic", "semantic", "procedural", "negative_pattern"],
                            "description": "Memory category (default: 'semantic')"
                        },
                        "scope": {
                            "type": "string",
                            "description": "Scope for the memory (e.g. 'project:my-project' or 'global', default: 'global')"
                        },
                        "tags": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "Categorical tags (e.g. ['database', 'architecture'])"
                        },
                        "importance": {
                            "type": "number",
                            "description": "Importance weight from 0.0 to 1.0 (default: 0.5)"
                        },
                        "confidence": {
                            "type": "number",
                            "description": "Confidence score from 0.0 to 1.0 (default: 1.0)"
                        }
                    },
                    "required": ["content"]
                }),
            },
            ToolDefinition {
                name: "memory_digest".to_string(),
                description: "Generate a compact digest (~300-500 tokens) of active project context, recent decisions, open threads, and known failure anti-patterns.".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "session_id": {
                            "type": "string",
                            "description": "Session ID or project scope to summarize (default: 'default')"
                        },
                        "max_tokens": {
                            "type": "integer",
                            "description": "Estimated maximum token limit for the digest (default: 500)"
                        }
                    }
                }),
            },
            ToolDefinition {
                name: "memory_feedback".to_string(),
                description: "Provide reinforcement feedback (positive/negative rating, confidence score, comments) on a persistent memory record to optimize cognitive ranking and recall accuracy.".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "id": {
                            "type": "string",
                            "description": "The UUID of the memory record to provide feedback on"
                        },
                        "rating": {
                            "type": "string",
                            "enum": ["positive", "negative"],
                            "description": "Reinforcement rating ('positive' or 'negative')"
                        },
                        "score": {
                            "type": "number",
                            "description": "Optional explicit confidence score (0.0 to 1.0)"
                        },
                        "comment": {
                            "type": "string",
                            "description": "Optional reasoning or feedback commentary"
                        }
                    },
                    "required": ["id", "rating"]
                }),
            },
            ToolDefinition {
                name: "causal_blast_radius".to_string(),
                description: "Analyze the architectural causal blast radius, downstream ripple effects, and breaking change risks before modifying code.".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "target": {
                            "type": "string",
                            "description": "File path, module name, struct or API to evaluate (e.g. 'crates/strata-server/src/storage.rs' or 'ServerStorage')"
                        },
                        "depth": {
                            "type": "integer",
                            "description": "Maximum traversal depth for transitive dependencies (default: 3)"
                        }
                    },
                    "required": ["target"]
                }),
            },
            ToolDefinition {
                name: "goal_decompose".to_string(),
                description: "Decompose a complex or long-horizon engineering objective into a structured Goal DAG with parallel waves, dependencies, and verification gates.".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "goal": {
                            "type": "string",
                            "description": "The natural language goal or objective to decompose"
                        },
                        "include_verification": {
                            "type": "boolean",
                            "description": "Whether to include verification gates (default: true)"
                        }
                    },
                    "required": ["goal"]
                }),
            },
            ToolDefinition {
                name: "dag_execute".to_string(),
                description: "Execute a Goal DAG plan wave-by-wave asynchronously with bounded concurrency, verification gate checks, and dynamic failure recovery.".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "goal": {
                            "type": "string",
                            "description": "Optional natural language goal to decompose and execute"
                        },
                        "dag": {
                            "type": "object",
                            "description": "Optional pre-decomposed Goal DAG JSON export to execute"
                        },
                        "max_concurrency": {
                            "type": "integer",
                            "description": "Maximum parallel concurrency during wave execution (default: 4)"
                        },
                        "auto_recover": {
                            "type": "boolean",
                            "description": "Whether to dynamically recover from failures by patching DAG (default: true)"
                        }
                    }
                }),
            },
            ToolDefinition {
                name: "train_pipeline".to_string(),
                description: "Synthesize one-click local LoRA fine-tuning scripts (Unsloth), datasets, and Ollama Modelfiles from continuous memory traces.".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "base_model": {
                            "type": "string",
                            "description": "HuggingFace base model identifier (default: 'unsloth/Llama-3.2-3B-Instruct')"
                        },
                        "method": {
                            "type": "string",
                            "enum": ["dpo", "sft", "orpo", "kto"],
                            "description": "Fine-tuning method (default: 'dpo')"
                        },
                        "output_dir": {
                            "type": "string",
                            "description": "Artifact output directory (default: './outputs/lora_run')"
                        },
                        "ollama_model_name": {
                            "type": "string",
                            "description": "Target Ollama model identifier (default: 'strata-custom-coder')"
                        },
                        "dry_run": {
                            "type": "boolean",
                            "description": "Whether to run in dry-run mode (default: true)"
                        }
                    }
                }),
            },
        ]
    }


    pub async fn run_stdio(self) -> anyhow::Result<()> {
        info!("Starting Strata MCP server on stdio transport");

        let stdin = tokio::io::stdin();
        let mut stdout = tokio::io::stdout();
        let mut reader = BufReader::new(stdin).lines();

        while let Some(line) = reader.next_line().await? {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }

            debug!("Received JSON-RPC message: {}", line);

            let req: JsonRpcRequest = match serde_json::from_str(line) {
                Ok(r) => r,
                Err(e) => {
                    error!("JSON-RPC parse error: {e}");
                    let resp = JsonRpcResponse::error(
                        serde_json::Value::Null,
                        JsonRpcError::parse_error(format!("Invalid JSON: {e}")),
                    );
                    let serialized = serde_json::to_string(&resp)?;
                    stdout.write_all(serialized.as_bytes()).await?;
                    stdout.write_all(b"\n").await?;
                    stdout.flush().await?;
                    continue;
                }
            };

            let is_notification = req.id.is_none();
            let req_id = req.id.clone().unwrap_or(serde_json::Value::Null);

            let response = self.handle_request(req).await;

            if !is_notification {
                if let Some(resp) = response {
                    let serialized = serde_json::to_string(&resp)?;
                    stdout.write_all(serialized.as_bytes()).await?;
                    stdout.write_all(b"\n").await?;
                    stdout.flush().await?;
                } else {
                    let resp = JsonRpcResponse::success(req_id, serde_json::json!({}));
                    let serialized = serde_json::to_string(&resp)?;
                    stdout.write_all(serialized.as_bytes()).await?;
                    stdout.write_all(b"\n").await?;
                    stdout.flush().await?;
                }
            }
        }

        info!("Strata MCP server stdio loop exited");
        Ok(())
    }

    pub async fn handle_request(&self, req: JsonRpcRequest) -> Option<JsonRpcResponse> {
        let req_id = req.id.clone().unwrap_or(serde_json::Value::Null);

        match req.method.as_str() {
            "initialize" => {
                let client_version = req.params.as_ref().and_then(|p| {
                    p.get("protocolVersion").and_then(|v| v.as_str())
                });
                let negotiated_version = negotiate_protocol_version(client_version);

                let init_result = InitializeResult {
                    protocol_version: negotiated_version.to_string(),
                    capabilities: ServerCapabilities {
                        tools: Some(ToolsCapability { list_changed: Some(false) }),
                        resources: None,
                        prompts: None,
                    },
                    server_info: ImplementationInfo {
                        name: self.server_name.clone(),
                        version: self.server_version.clone(),
                    },
                    meta: Some(serde_json::json!({
                        "ttlMs": 3600000,
                        "cacheScope": "session"
                    })),
                };
                Some(JsonRpcResponse::success(
                    req_id,
                    serde_json::to_value(init_result).unwrap_or(serde_json::json!({})),
                ))
            }
            "notifications/initialized" | "initialized" => {
                debug!("Client initialized notification received");
                None
            }
            "ping" => Some(JsonRpcResponse::success(req_id, serde_json::json!({}))),
            "tools/list" => {
                let tools_result = ToolsListResult::new(Self::tool_definitions());
                Some(JsonRpcResponse::success(
                    req_id,
                    serde_json::to_value(tools_result).unwrap_or(serde_json::json!({})),
                ))
            }

            "tools/call" => {
                let params: CallToolRequestParams = match req.params {
                    Some(p) => match serde_json::from_value(p) {
                        Ok(call_params) => call_params,
                        Err(e) => {
                            return Some(JsonRpcResponse::error(
                                req_id,
                                JsonRpcError::invalid_params(format!("Invalid tools/call params: {e}")),
                            ));
                        }
                    },
                    None => {
                        return Some(JsonRpcResponse::error(
                            req_id,
                            JsonRpcError::invalid_params("Missing tools/call params"),
                        ));
                    }
                };

                let call_result = self.execute_tool(&params.name, params.arguments.unwrap_or(serde_json::json!({}))).await;
                Some(JsonRpcResponse::success(
                    req_id,
                    serde_json::to_value(call_result).unwrap_or(serde_json::json!({})),
                ))
            }
            unknown_method => {
                if req.id.is_some() {
                    Some(JsonRpcResponse::error(
                        req_id,
                        JsonRpcError::method_not_found(unknown_method),
                    ))
                } else {
                    None
                }
            }
        }
    }

    pub async fn execute_tool(&self, name: &str, args: serde_json::Value) -> CallToolResult {
        match name {
            "memory_search" => {
                let query = match args.get("query").and_then(|v| v.as_str()) {
                    Some(q) => q,
                    None => return CallToolResult::error("Missing required parameter: query"),
                };

                let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(5) as usize;

                let scope = args
                    .get("scope")
                    .and_then(|v| v.as_str())
                    .map(|s| s.parse::<Scope>().unwrap_or(Scope::Global));

                let memory_type = args
                    .get("memory_type")
                    .and_then(|v| v.as_str())
                    .and_then(|s| s.parse::<MemoryType>().ok());

                match self.memory_engine.search(query, scope.as_ref(), limit).await {
                    Ok(records) => {
                        let filtered_records: Vec<_> = if let Some(mt) = memory_type {
                            records.into_iter().filter(|r| r.memory_type == mt).collect()
                        } else {
                            records
                        };

                        if filtered_records.is_empty() {
                            CallToolResult::text(format!("No memories found matching query: \"{query}\""))
                        } else {
                            let handles: Vec<_> = filtered_records.iter().map(|r| r.to_handle(None)).collect();
                            match serde_json::to_string_pretty(&handles) {
                                Ok(json) => CallToolResult::text(json),
                                Err(e) => CallToolResult::error(format!("Failed to serialize search results: {e}")),
                            }
                        }
                    }
                    Err(e) => CallToolResult::error(format!("Memory search error: {e}")),
                }
            }
            "memory_get" => {
                let id_str = match args.get("id").and_then(|v| v.as_str()) {
                    Some(id) => id,
                    None => return CallToolResult::error("Missing required parameter: id"),
                };

                let uuid = match Uuid::parse_str(id_str) {
                    Ok(u) => u,
                    Err(e) => return CallToolResult::error(format!("Invalid UUID format: {e}")),
                };

                match self.memory_engine.get(&uuid).await {
                    Ok(Some(record)) => match serde_json::to_string_pretty(&record) {
                        Ok(json) => CallToolResult::text(json),
                        Err(e) => CallToolResult::error(format!("Serialization error: {e}")),
                    },
                    Ok(None) => CallToolResult::text(format!("Memory record with ID '{id_str}' not found.")),
                    Err(e) => CallToolResult::error(format!("Memory get error: {e}")),
                }
            }
            "memory_write" => {
                let content = match args.get("content").and_then(|v| v.as_str()) {
                    Some(c) => c,
                    None => return CallToolResult::error("Missing required parameter: content"),
                };

                let summary = args.get("summary").and_then(|v| v.as_str());

                let memory_type = args
                    .get("memory_type")
                    .and_then(|v| v.as_str())
                    .and_then(|s| s.parse::<MemoryType>().ok())
                    .unwrap_or(MemoryType::Semantic);

                let scope = args
                    .get("scope")
                    .and_then(|v| v.as_str())
                    .map(|s| s.parse::<Scope>().unwrap_or(Scope::Global))
                    .unwrap_or(Scope::Global);

                let importance = args
                    .get("importance")
                    .and_then(|v| v.as_f64())
                    .map(|f| f as f32)
                    .unwrap_or(0.5);

                let confidence = args
                    .get("confidence")
                    .and_then(|v| v.as_f64())
                    .map(|f| f as f32)
                    .unwrap_or(1.0);

                let tags: Vec<String> = args
                    .get("tags")
                    .and_then(|v| v.as_array())
                    .map(|arr| arr.iter().filter_map(|t| t.as_str().map(String::from)).collect())
                    .unwrap_or_default();

                let mut record = MemoryRecord::new(memory_type, content, scope)
                    .with_importance(importance)
                    .with_confidence(confidence)
                    .with_tags(tags);

                if let Some(s) = summary {
                    record = record.with_summary(s);
                }

                match self.memory_engine.write(&record).await {
                    Ok(handle) => match serde_json::to_string_pretty(&serde_json::json!({
                        "status": "stored",
                        "id": handle.id.to_string(),
                        "title": handle.title,
                        "memory_type": handle.memory_type.to_string(),
                        "scope": handle.scope.to_string()
                    })) {
                        Ok(json) => CallToolResult::text(json),
                        Err(e) => CallToolResult::error(format!("Serialization error: {e}")),
                    },
                    Err(e) => CallToolResult::error(format!("Memory write error: {e}")),
                }
            }
            "memory_digest" => {
                let session_id = args
                    .get("session_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("default");

                let max_tokens = args.get("max_tokens").and_then(|v| v.as_u64()).map(|t| t as usize);

                match self.memory_engine.digest(session_id, max_tokens).await {
                    Ok(digest) => match serde_json::to_string_pretty(&digest) {
                        Ok(json) => CallToolResult::text(json),
                        Err(e) => CallToolResult::error(format!("Serialization error: {e}")),
                    },
                    Err(e) => CallToolResult::error(format!("Memory digest error: {e}")),
                }
            }
            "memory_feedback" => {
                let id_str = match args.get("id").and_then(|v| v.as_str()) {
                    Some(id) => id,
                    None => return CallToolResult::error("Missing required parameter: id"),
                };

                let rating = match args.get("rating").and_then(|v| v.as_str()) {
                    Some(r) => r,
                    None => return CallToolResult::error("Missing required parameter: rating"),
                };

                let score = args.get("score").and_then(|v| v.as_f64()).map(|s| s as f32);
                let comment = args.get("comment").and_then(|v| v.as_str());

                let uuid = match Uuid::parse_str(id_str) {
                    Ok(u) => u,
                    Err(e) => return CallToolResult::error(format!("Invalid UUID format: {e}")),
                };

                match self.memory_engine.get(&uuid).await {
                    Ok(Some(mut record)) => {
                        let new_confidence = if let Some(s) = score {
                            s.clamp(0.0, 1.0)
                        } else if rating.eq_ignore_ascii_case("positive") {
                            (record.confidence + 0.1).min(1.0)
                        } else if rating.eq_ignore_ascii_case("negative") {
                            (record.confidence - 0.2).max(0.0)
                        } else {
                            record.confidence
                        };

                        record.confidence = new_confidence;
                        if let Some(c) = comment {
                            if let Some(meta_obj) = record.metadata.as_object_mut() {
                                meta_obj.insert("last_feedback_comment".to_string(), serde_json::Value::String(c.to_string()));
                            } else {
                                record.metadata = serde_json::json!({ "last_feedback_comment": c });
                            }
                        }

                        match self.memory_engine.write(&record).await {
                            Ok(handle) => {
                                let structured = serde_json::json!({
                                    "status": "feedback_recorded",
                                    "id": handle.id.to_string(),
                                    "rating": rating,
                                    "confidence": new_confidence,
                                    "comment": comment
                                });
                                CallToolResult::structured(
                                    format!("Feedback recorded for memory '{}': rating='{}', new confidence={:.2}", handle.id, rating, new_confidence),
                                    structured,
                                )
                            }
                            Err(e) => CallToolResult::error(format!("Failed to record memory feedback: {e}")),
                        }
                    }
                    Ok(None) => CallToolResult::error(format!("Memory record with ID '{id_str}' not found")),
                    Err(e) => CallToolResult::error(format!("Memory get error: {e}")),
                }
            }
            "causal_blast_radius" => {
                let target = match args.get("target").and_then(|v| v.as_str()) {
                    Some(t) => t,
                    None => return CallToolResult::error("Missing required parameter: target"),
                };

                let depth = args.get("depth").and_then(|v| v.as_u64()).unwrap_or(3) as usize;

                let world_model = strata_reasoning::WorldModel::new();
                let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
                let _ = world_model.index_workspace(&cwd).await;

                match world_model.predict_impact(target, depth).await {
                    Ok(report) => {
                        let text_tree = world_model.to_ascii_tree(target, depth).await.unwrap_or_default();
                        let structured = serde_json::to_value(&report).unwrap_or(serde_json::json!({}));
                        CallToolResult::structured(text_tree, structured)
                    }
                    Err(e) => CallToolResult::error(format!("Causal blast radius prediction error: {e}")),
                }
            }
            "goal_decompose" => {
                let goal = match args.get("goal").and_then(|v| v.as_str()) {
                    Some(g) => g,
                    None => return CallToolResult::error("Missing required parameter: goal"),
                };

                let include_verification = args
                    .get("include_verification")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(true);

                let decomposer = strata_reasoning::GoalDecomposer::new()
                    .with_verification_gates(include_verification);

                match decomposer.decompose(goal) {
                    Ok(dag) => {
                        let text_tree = dag.to_ascii_tree();
                        let structured = serde_json::json!({
                            "status": "success",
                            "goal": goal,
                            "total_nodes": dag.node_count(),
                            "dag": dag.export(),
                            "waves": dag.compute_waves().unwrap_or_default()
                        });
                        CallToolResult::structured(text_tree, structured)
                    }
                    Err(e) => CallToolResult::error(format!("Goal decomposition error: {e}")),
                }
            }
            "dag_execute" => {
                let concurrency = args
                    .get("max_concurrency")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(4) as usize;

                let auto_recover = args
                    .get("auto_recover")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(true);

                let dag = if let Some(dag_val) = args.get("dag") {
                    match serde_json::from_value::<strata_reasoning::GoalDagExport>(dag_val.clone()) {
                        Ok(export) => match strata_reasoning::GoalDag::from_export(export) {
                            Ok(d) => d,
                            Err(e) => return CallToolResult::error(format!("Invalid Goal DAG structure: {e}")),
                        },
                        Err(e) => return CallToolResult::error(format!("Invalid Goal DAG export format: {e}")),
                    }
                } else if let Some(goal) = args.get("goal").and_then(|v| v.as_str()) {
                    match strata_reasoning::GoalDecomposer::new().decompose(goal) {
                        Ok(d) => d,
                        Err(e) => return CallToolResult::error(format!("Goal decomposition error: {e}")),
                    }
                } else {
                    return CallToolResult::error("Either 'goal' or 'dag' parameter must be provided");
                };

                let scheduler = strata_reasoning::DagScheduler::new()
                    .with_concurrency(concurrency)
                    .with_auto_recover(auto_recover);

                match scheduler.execute(dag).await {
                    Ok((finished_dag, report)) => {
                        let text_tree = finished_dag.to_ascii_tree();
                        let structured = serde_json::to_value(&report).unwrap_or(serde_json::json!({}));
                        CallToolResult::structured(text_tree, structured)
                    }
                    Err(e) => CallToolResult::error(format!("DAG execution error: {e}")),
                }
            }
            "train_pipeline" => {
                use strata_core::traits::Tool;
                let tool = strata_tools::TrainPipelineTool::new();
                match tool.execute(args).await {
                    Ok(val) => {
                        let summary_table = val
                            .get("summary_table")
                            .and_then(|v| v.as_str())
                            .unwrap_or("Training pipeline execution completed successfully.");
                        CallToolResult::structured(summary_table.to_string(), val)
                    }
                    Err(e) => CallToolResult::error(format!("Train pipeline error: {e}")),
                }
            }
            unknown_tool => CallToolResult::error(format!("Unknown tool: '{unknown_tool}'")),
        }
    }
}

