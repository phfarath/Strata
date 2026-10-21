use chrono::{DateTime, Utc};
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tracing::{debug, error, info};
use uuid::Uuid;

use strata_core::{
    a2a::AgentPresence,
    schemas::{EpisodeRecord, EpisodeStep},
    state::{MemoryRecord, MemoryType, Scope},
    traits::MemoryEngine,
};
use strata_memory::{
    EpisodicPyramidEngine, EpisodicPyramidStore, PyramidConfig, PyramidFilter, SqliteMemoryEngine,
    StigmergyCoordinator,
};

use super::protocol::*;

pub struct McpServer {
    memory_engine: Arc<dyn MemoryEngine>,
    sqlite_engine: Option<Arc<SqliteMemoryEngine>>,
    stigmergy: Option<StigmergyCoordinator>,
    ipc_client: Option<Arc<strata_memory::IpcClient>>,
    _ipc_server: Option<Arc<strata_memory::IpcServer>>,
    server_name: String,
    server_version: String,
}

impl McpServer {
    pub fn new(memory_engine: Arc<dyn MemoryEngine>) -> Self {
        Self {
            memory_engine,
            sqlite_engine: None,
            stigmergy: None,
            ipc_client: None,
            _ipc_server: None,
            server_name: "strata-mcp".to_string(),
            server_version: env!("CARGO_PKG_VERSION").to_string(),
        }
    }

    pub fn new_with_engine(engine: Arc<SqliteMemoryEngine>) -> Self {
        let coordinator = engine.stigmergy();
        Self {
            memory_engine: Arc::clone(&engine) as Arc<dyn MemoryEngine>,
            sqlite_engine: Some(Arc::clone(&engine)),
            stigmergy: Some(coordinator),
            ipc_client: None,
            _ipc_server: None,
            server_name: "strata-mcp".to_string(),
            server_version: env!("CARGO_PKG_VERSION").to_string(),
        }
    }

    pub fn with_sqlite_engine(mut self, engine: Arc<SqliteMemoryEngine>) -> Self {
        self.sqlite_engine = Some(engine);
        self
    }

    pub fn with_stigmergy(mut self, coordinator: StigmergyCoordinator) -> Self {
        self.stigmergy = Some(coordinator);
        self
    }

    pub fn with_ipc(
        mut self,
        client: Option<Arc<strata_memory::IpcClient>>,
        server: Option<Arc<strata_memory::IpcServer>>,
    ) -> Self {
        self.ipc_client = client;
        self._ipc_server = server;
        self
    }

    pub async fn init_workspace_ipc(&mut self, workspace_root: &std::path::Path) {
        let (client, server) =
            strata_memory::IpcBrokerManager::start_or_connect(workspace_root).await;
        self.ipc_client = client;
        self._ipc_server = server;
    }

    pub fn tool_definitions() -> Vec<ToolDefinition> {
        let mut defs = vec![
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
                        },
                        "graph_recall": {
                            "type": "boolean",
                            "description": "Whether to augment retrieval with associative Knowledge Graph spreading activation (default: true)"
                        },
                        "as_of_time": {
                            "type": "string",
                            "description": "Optional ISO 8601 / RFC 3339 timestamp for historical bi-temporal time-travel queries (e.g. '2026-08-01T12:00:00Z')"
                        },
                        "as_of_commit": {
                            "type": "string",
                            "description": "Optional Git commit hash (SHA) to anchor historical playback queries"
                        },
                        "as_of_branch": {
                            "type": "string",
                            "description": "Optional Git branch namespace filter (e.g. 'main', 'feat/xyz'). Defaults to current branch if detected."
                        }
                    },
                    "required": ["query"]
                }),
            },
            ToolDefinition {
                name: "memory_graph_explore".to_string(),
                description: "Explore the associative Knowledge Graph (HippoRAG / ACT-R style) starting from a seed term (symbol name, file path, concept tag, or memory UUID). Traverses call edges, file anchors, and justification links to reveal connected architecture context with 0 tokens.".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "seed": {
                            "type": "string",
                            "description": "Seed term to explore: memory UUID, symbol name, file path, or concept tag"
                        },
                        "limit": {
                            "type": "integer",
                            "description": "Maximum number of connected memories to return (default: 5)"
                        },
                        "scope": {
                            "type": "string",
                            "description": "Optional scope filter: 'global', 'project:<name>', 'session:<id>'"
                        }
                    },
                    "required": ["seed"]
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
                        },
                        "as_of_time": {
                            "type": "string",
                            "description": "Optional ISO 8601 / RFC 3339 timestamp for historical bi-temporal time-travel queries"
                        },
                        "as_of_commit": {
                            "type": "string",
                            "description": "Optional Git commit hash (SHA) to anchor historical playback queries"
                        },
                        "as_of_branch": {
                            "type": "string",
                            "description": "Optional Git branch namespace filter (e.g. 'main', 'feat/xyz')"
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
            ToolDefinition {
                name: "distill_pipeline".to_string(),
                description: "Synthesize self-supervised distillation curriculum, train/eval dataset splits, Unsloth LoRA scripts, and Ollama Modelfile from continuous memory and oracle-verified delegations.".to_string(),
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
                        "eval_ratio": {
                            "type": "number",
                            "description": "Validation evaluation split ratio (default: 0.1)"
                        },
                        "oracle_only": {
                            "type": "boolean",
                            "description": "Gate to oracle-verified traces only (default: true)"
                        },
                        "output_dir": {
                            "type": "string",
                            "description": "Artifact output directory (default: './outputs/distill_run')"
                        },
                        "ollama_model_name": {
                            "type": "string",
                            "description": "Target Ollama model identifier (default: 'strata-custom-distilled')"
                        },
                        "dry_run": {
                            "type": "boolean",
                            "description": "Whether to run in dry-run mode (default: true)"
                        }
                    }
                }),
            },
            ToolDefinition {
                name: "strata_call_graph".to_string(),
                description: "Deterministic Native Call Graph and Module Import Dependency Analyzer in Rust. Queries callers, callees, imports, and recursive cycles for any symbol or file.".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "File path to analyze or query"
                        },
                        "code": {
                            "type": "string",
                            "description": "Optional raw source code string to analyze on-the-fly"
                        },
                        "symbol": {
                            "type": "string",
                            "description": "Optional function/symbol name to inspect callers or callees"
                        },
                        "direction": {
                            "type": "string",
                            "enum": ["callers", "callees", "both", "imports", "all"],
                            "description": "Call hierarchy direction to retrieve (default: 'all')"
                        },
                        "limit": {
                            "type": "integer",
                            "description": "Maximum number of edges to return (default: 50)"
                        }
                    }
                }),
            },
            ToolDefinition {
                name: "memory_index_workspace".to_string(),
                description: "Deep Heuristic AST & Call Graph Indexer. Recursively scans workspace source files, extracts AST symbols and call edges, resolves cross-file caller-callee bindings, and persists both call edges and associative Knowledge Graph edges into SQLite.".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "Root workspace directory or source file to index (default: '.')"
                        }
                    }
                }),
            },
            ToolDefinition {
                name: "strata_workspace_detect".to_string(),
                description: "Detect monorepo workspace boundaries, member packages/crates, internal dependencies, and isolate file search scopes.".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "root_path": {
                            "type": "string",
                            "description": "Root directory of the workspace or monorepo to inspect (default: '.')"
                        },
                        "file_path": {
                            "type": "string",
                            "description": "Optional file path to resolve to its owner package and compute hierarchical scopes"
                        }
                    }
                }),
            },
            ToolDefinition {
                name: "strata_architecture_map".to_string(),
                description: "Extract graph communities and high-level architectural clusters from codebase call graphs, imports, and AST symbols to understand macro software architecture.".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "Root workspace or directory path to analyze (default: '.')"
                        },
                        "workspace_id": {
                            "type": "string",
                            "description": "Workspace identifier (default: 'default')"
                        },
                        "min_cluster_size": {
                            "type": "integer",
                            "description": "Minimum member symbols/files required per cluster (default: 1)"
                        },
                        "max_iterations": {
                            "type": "integer",
                            "description": "Maximum LPA convergence iterations (default: 25)"
                        }
                    }
                }),
            },
            ToolDefinition {
                name: "strata_promote".to_string(),
                description: "Promote a memory record or semantic fact to permanent Core Tier (frozen retention, R=1.0). Requires a verifiable human approval token generated via `strata promote <id> --generate-token`.".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "id": {
                            "type": "string",
                            "description": "The UUID of the memory record or semantic fact to promote"
                        },
                        "approval_token": {
                            "type": "string",
                            "description": "Cryptographically secure, single-use human approval token generated via `strata promote <id> --generate-token`"
                        },
                        "entity_type": {
                            "type": "string",
                            "enum": ["memory", "fact"],
                            "description": "Entity type: 'memory' (default) or 'fact'"
                        },
                        "reason": {
                            "type": "string",
                            "description": "Optional policy rationale or justification for permanent Core Tier promotion"
                        }
                    },
                    "required": ["id", "approval_token"]
                }),
            },
            ToolDefinition {
                name: "lease_acquire".to_string(),
                description: "Atomically acquire or renew an exclusive temporal lease on a workspace resource (e.g. 'crate:strata-cli', 'file:src/mcp/server.rs') to prevent cross-agent collisions. Auto-expires with TTL if process terminates.".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "resource_id": {
                            "type": "string",
                            "description": "Unique identifier of the resource to lease (e.g. 'crate:strata-cli', 'file:src/main.rs')"
                        },
                        "agent_id": {
                            "type": "string",
                            "description": "Identifier of the requesting agent (e.g. 'cursor', 'claude-code')"
                        },
                        "ttl_seconds": {
                            "type": "integer",
                            "description": "Duration of lease in seconds before auto-expiration (default: 30)"
                        },
                        "metadata": {
                            "type": "string",
                            "description": "Optional description of the task or modification planned"
                        }
                    },
                    "required": ["resource_id", "agent_id"]
                }),
            },
            ToolDefinition {
                name: "lease_release".to_string(),
                description: "Explicitly release an exclusive temporal lease on a workspace resource upon task completion.".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "resource_id": {
                            "type": "string",
                            "description": "Resource ID to release"
                        },
                        "agent_id": {
                            "type": "string",
                            "description": "Agent identifier that holds the lease"
                        }
                    },
                    "required": ["resource_id", "agent_id"]
                }),
            },
            ToolDefinition {
                name: "agent_who".to_string(),
                description: "List all active agents currently working in the workspace, their hosts, active tasks, and active resource leases (Stigmergic Workspace Awareness).".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "ttl_seconds": {
                            "type": "integer",
                            "description": "Freshness window in seconds for active agent presence (default: 60)"
                        }
                    }
                }),
            },
            ToolDefinition {
                name: "agent_heartbeat".to_string(),
                description: "Register or refresh an agent's presence, process ID, and active task in the shared workspace state.".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "agent_id": {
                            "type": "string",
                            "description": "Identifier of the agent (e.g. 'cursor-session-1', 'claude-terminal')"
                        },
                        "host": {
                            "type": "string",
                            "description": "Host environment ('cursor', 'claude-code', 'gemini-cli', 'windsurf', etc.)"
                        },
                        "pid": {
                            "type": "integer",
                            "description": "Process ID of the agent (default: 0)"
                        },
                        "active_task": {
                            "type": "string",
                            "description": "Optional headline of the task the agent is actively executing"
                        }
                    },
                    "required": ["agent_id", "host"]
                }),
            },
            ToolDefinition {
                name: "memory_consolidate".to_string(),
                description: "Consolidate episodic event stream into semantic facts (with JTMS contradiction arbitration), procedural recovery skills, and episodic memory clusters using deterministic Neuro-Symbolic consolidation (0 tokens offline, optional LLM enrichment).".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "session_id": {
                            "type": "string",
                            "description": "Optional session ID to consolidate (default: 'default')"
                        },
                        "all_sessions": {
                            "type": "boolean",
                            "description": "Whether to consolidate across all recorded workspace sessions (default: false)"
                        },
                        "enrich_with_llm": {
                            "type": "boolean",
                            "description": "Whether to run decoupled background LLM enrichment on top of deterministic facts (default: false)"
                        }
                    }
                }),
            },
            ToolDefinition {
                name: "memory_promote_global".to_string(),
                description: "Promote a validated memory record or negative pattern to the Developer-Global store (~/.strata/global.db) with Scope::Global. Requires a verifiable human approval token generated via `strata promote <id> --to-global --generate-token`.".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "id": {
                            "type": "string",
                            "description": "The UUID of the memory record to promote to Global Store"
                        },
                        "approval_token": {
                            "type": "string",
                            "description": "Cryptographically secure, single-use human approval token generated via `strata promote <id> --to-global --generate-token`"
                        },
                        "reason": {
                            "type": "string",
                            "description": "Optional policy rationale or context for global cross-project promotion"
                        }
                    },
                    "required": ["id", "approval_token"]
                }),
            },
            ToolDefinition {
                name: "episode_record".to_string(),
                description: "Record a multi-step episodic execution trace with action steps, observations, rewards, salience score, and outcomes for subsequent replay and hippocampal consolidation.".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "summary": {
                            "type": "string",
                            "description": "High-level summary of the episodic execution trace"
                        },
                        "session_id": {
                            "type": "string",
                            "description": "Session ID (default: 'default')"
                        },
                        "agent_id": {
                            "type": "string",
                            "description": "Agent ID that executed the steps (default: 'local-agent')"
                        },
                        "task_id": {
                            "type": "string",
                            "description": "Optional task ID on the blackboard"
                        },
                        "salience_score": {
                            "type": "number",
                            "description": "Salience score (0.0 to 1.0, default: 0.5)"
                        },
                        "outcomes": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "List of observed outcomes or assertions"
                        },
                        "steps": {
                            "type": "array",
                            "items": {
                                "type": "object",
                                "properties": {
                                    "action": { "type": "string" },
                                    "observation": { "type": "string" },
                                    "reward": { "type": "number" },
                                    "error": { "type": "string" }
                                },
                                "required": ["action", "observation"]
                            },
                            "description": "Sequential steps executed during the episode"
                        },
                        "causal_provenance": {
                            "type": "string",
                            "description": "Optional causal provenance or environment context"
                        }
                    },
                    "required": ["summary", "steps"]
                }),
            },
            ToolDefinition {
                name: "episode_replay".to_string(),
                description: "Replay and evaluate an episodic trace against expectations, computing simulated reward and extracting candidate semantic facts and anti-patterns.".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "id": {
                            "type": "string",
                            "description": "UUID of the episode record to replay"
                        }
                    },
                    "required": ["id"]
                }),
            },
            ToolDefinition {
                name: "episode_consolidate".to_string(),
                description: "Run hippocampal consolidation over recorded episodic traces: synthesizes semantic facts, performs JTMS contradiction arbitration, and promotes durable insights to Core or Working memory tiers.".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "session_id": {
                            "type": "string",
                            "description": "Optional session ID to consolidate (omitted = consolidate all pending episodes)"
                        }
                    }
                }),
            },
            ToolDefinition {
                name: "memory_episode_pyramid".to_string(),
                description: "Hierarchical Episodic Memory Pyramid operations: 'rollup' (aggregates Tier 1 traces into Tier 2 milestones), 'zoom_out' (retrieves top-level Tier 2 milestone rollups), 'drill_down' (reconstructs hierarchical execution tree for a specific rollup), or 'tree' (inspects full hierarchy across rollups).".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "action": {
                            "type": "string",
                            "enum": ["rollup", "zoom_out", "drill_down", "tree"],
                            "description": "Pyramid operation to perform: 'rollup', 'zoom_out', 'drill_down', or 'tree'"
                        },
                        "session_id": {
                            "type": "string",
                            "description": "Optional session ID filter"
                        },
                        "rollup_id": {
                            "type": "string",
                            "description": "UUID of the rollup episode (required for 'drill_down')"
                        },
                        "limit": {
                            "type": "integer",
                            "description": "Maximum rollups to return for 'zoom_out' or 'tree' (default: 20)"
                        }
                    },
                    "required": ["action"]
                }),
            },
            ToolDefinition {
                name: "memory_pyramid_drilldown".to_string(),
                description: "Hierarchical Episodic Pyramid drill-down tool. Returns compact Tier 2 milestone summaries with progressive on-demand expansion of raw Tier 1 episode traces. Filters by cluster_id, rollup_id, or temporal interval (start_time/end_time/since_seconds) to prevent LLM context explosion.".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "rollup_id": {
                            "type": "string",
                            "description": "Optional UUID of a specific Tier 2 milestone rollup to inspect"
                        },
                        "cluster_id": {
                            "type": "string",
                            "description": "Optional cluster or task ID filter (e.g. 'task-auth-jwt' or cluster UUID)"
                        },
                        "start_time": {
                            "type": "integer",
                            "description": "Optional start timestamp in Unix epoch seconds"
                        },
                        "end_time": {
                            "type": "integer",
                            "description": "Optional end timestamp in Unix epoch seconds"
                        },
                        "since_seconds": {
                            "type": "integer",
                            "description": "Optional relative time window: only return rollups created within the last N seconds"
                        },
                        "session_id": {
                            "type": "string",
                            "description": "Optional session ID filter"
                        },
                        "expand_episodes": {
                            "type": "boolean",
                            "description": "Whether to expand raw Tier 1 leaf episodes and step traces on demand (default: false, returns compact Tier 2 summaries only to conserve context)"
                        },
                        "limit": {
                            "type": "integer",
                            "description": "Maximum number of rollups to return (default: 20)"
                        }
                    }
                }),
            },
        ];
        defs.extend(super::blackboard::definitions());
        defs
    }

    pub async fn run_stdio(self) -> anyhow::Result<()> {
        info!("Starting Strata MCP server on stdio transport");

        if let Some(ref client) = self.ipc_client {
            let mut sub = client.subscribe();
            tokio::spawn(async move {
                while let Ok(event) = sub.recv().await {
                    match event {
                        strata_core::a2a::IpcEvent::AntiPatternDiscovered {
                            category,
                            pattern,
                            remedy,
                            ..
                        } => {
                            tracing::warn!("⚡ [A2A Instant Alert] Peer agent discovered anti-pattern ({category}): {pattern} -> Remedy: {remedy}");
                        }
                        strata_core::a2a::IpcEvent::LeaseAcquired {
                            resource_id,
                            agent_id,
                            expires_at,
                            ..
                        } => {
                            tracing::info!("🔒 [A2A Stigmergy] Resource '{resource_id}' leased by '{agent_id}' until {expires_at}");
                        }
                        strata_core::a2a::IpcEvent::LeaseReleased {
                            resource_id,
                            agent_id,
                            ..
                        } => {
                            tracing::info!("🔓 [A2A Stigmergy] Resource '{resource_id}' released by '{agent_id}'");
                        }
                        _ => {}
                    }
                }
            });
        }

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
                let client_version = req
                    .params
                    .as_ref()
                    .and_then(|p| p.get("protocolVersion").and_then(|v| v.as_str()));
                let negotiated_version = negotiate_protocol_version(client_version);

                let init_result = InitializeResult {
                    protocol_version: negotiated_version.to_string(),
                    capabilities: ServerCapabilities {
                        tools: Some(ToolsCapability {
                            list_changed: Some(false),
                        }),
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
                                JsonRpcError::invalid_params(format!(
                                    "Invalid tools/call params: {e}"
                                )),
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

                let call_result = self
                    .execute_tool(
                        &params.name,
                        params.arguments.unwrap_or(serde_json::json!({})),
                    )
                    .await;
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

                let as_of_time = match args.get("as_of_time").and_then(|v| v.as_str()) {
                    Some(s) => match DateTime::parse_from_rfc3339(s) {
                        Ok(dt) => Some(dt.with_timezone(&Utc)),
                        Err(e) => {
                            return CallToolResult::error(format!(
                                "Invalid RFC3339 timestamp for as_of_time '{s}': {e}"
                            ))
                        }
                    },
                    None => None,
                };

                let as_of_commit = args.get("as_of_commit").and_then(|v| v.as_str());
                let as_of_branch = args.get("as_of_branch").and_then(|v| v.as_str());

                match self
                    .memory_engine
                    .search_as_of(
                        query,
                        scope.as_ref(),
                        as_of_time,
                        as_of_commit,
                        as_of_branch,
                        limit,
                    )
                    .await
                {
                    Ok(records) => {
                        let filtered_records: Vec<_> = if let Some(mt) = memory_type {
                            records
                                .into_iter()
                                .filter(|r| r.memory_type == mt)
                                .collect()
                        } else {
                            records
                        };

                        if filtered_records.is_empty() {
                            CallToolResult::text(format!(
                                "No memories found matching query: \"{query}\""
                            ))
                        } else {
                            if let Some(ref engine) = self.sqlite_engine {
                                let session_id = args
                                    .get("session_id")
                                    .and_then(|v| v.as_str())
                                    .map(String::from)
                                    .or_else(|| std::env::var("STRATA_SESSION_ID").ok())
                                    .unwrap_or_else(|| "default".to_string());
                                let task_id = args
                                    .get("task_id")
                                    .and_then(|v| v.as_str())
                                    .map(String::from)
                                    .or_else(|| std::env::var("STRATA_TASK_ID").ok());

                                for r in &filtered_records {
                                    let mut inj = strata_memory::MemoryInjectionRecord::new(
                                        r.id,
                                        &session_id,
                                        strata_memory::InjectionOutcome::Success,
                                    );
                                    if let Some(ref tid) = task_id {
                                        inj = inj.with_task_id(tid.clone());
                                    }
                                    let _ = engine.store().record_injection_outcome(&inj);
                                }
                            }

                            let handles: Vec<_> = filtered_records
                                .iter()
                                .map(|r| {
                                    let mut handle = r.to_handle(None);
                                    let prov_reason = format!("Matched search query: \"{query}\"");
                                    handle.provenance =
                                        Some(r.build_provenance(Some(&prov_reason)));
                                    let mut val = serde_json::to_value(&handle)
                                        .unwrap_or(serde_json::json!({}));
                                    if let Some(obj) = val.as_object_mut() {
                                        let is_global = if let Some(ref engine) = self.sqlite_engine
                                        {
                                            engine
                                                .store()
                                                .get_memory(&r.id)
                                                .ok()
                                                .flatten()
                                                .is_none()
                                        } else {
                                            false
                                        };
                                        obj.insert(
                                            "store_origin".to_string(),
                                            serde_json::json!(if is_global {
                                                "global"
                                            } else {
                                                "local"
                                            }),
                                        );
                                    }
                                    val
                                })
                                .collect();
                            match serde_json::to_string_pretty(&handles) {
                                Ok(json) => CallToolResult::text(json),
                                Err(e) => CallToolResult::error(format!(
                                    "Failed to serialize search results: {e}"
                                )),
                            }
                        }
                    }
                    Err(e) => CallToolResult::error(format!("Memory search error: {e}")),
                }
            }
            "memory_graph_explore" => {
                let seed = match args.get("seed").and_then(|v| v.as_str()) {
                    Some(s) => s,
                    None => return CallToolResult::error("Missing required parameter: seed"),
                };

                let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(5) as usize;
                let scope = args
                    .get("scope")
                    .and_then(|v| v.as_str())
                    .map(|s| s.parse::<Scope>().unwrap_or(Scope::Global));

                if let Some(engine) = &self.sqlite_engine {
                    match engine.search_graph_associative(seed, scope.as_ref(), limit) {
                        Ok(records_with_scores) => {
                            if records_with_scores.is_empty() {
                                CallToolResult::text(format!(
                                    "No associative connections found for seed: \"{seed}\""
                                ))
                            } else {
                                let output: Vec<serde_json::Value> = records_with_scores
                                    .into_iter()
                                    .map(|(rec, score)| {
                                        let mut val = serde_json::to_value(rec.to_handle(None))
                                            .unwrap_or(serde_json::json!({}));
                                        if let Some(obj) = val.as_object_mut() {
                                            obj.insert(
                                                "spreading_activation_score".to_string(),
                                                serde_json::json!(score),
                                            );
                                        }
                                        val
                                    })
                                    .collect();
                                match serde_json::to_string_pretty(&output) {
                                    Ok(json) => CallToolResult::text(json),
                                    Err(e) => CallToolResult::error(format!(
                                        "Failed to serialize graph explore results: {e}"
                                    )),
                                }
                            }
                        }
                        Err(e) => CallToolResult::error(format!("Graph explore error: {e}")),
                    }
                } else {
                    CallToolResult::error("Knowledge Graph explore requires SQLite engine")
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

                let as_of_time = match args.get("as_of_time").and_then(|v| v.as_str()) {
                    Some(s) => match DateTime::parse_from_rfc3339(s) {
                        Ok(dt) => Some(dt.with_timezone(&Utc)),
                        Err(e) => {
                            return CallToolResult::error(format!(
                                "Invalid RFC3339 timestamp for as_of_time '{s}': {e}"
                            ))
                        }
                    },
                    None => None,
                };

                let as_of_commit = args.get("as_of_commit").and_then(|v| v.as_str());
                let as_of_branch = args.get("as_of_branch").and_then(|v| v.as_str());

                match self
                    .memory_engine
                    .get_as_of(&uuid, as_of_time, as_of_commit, as_of_branch)
                    .await
                {
                    Ok(Some(record)) => {
                        let mut val =
                            serde_json::to_value(&record).unwrap_or(serde_json::json!({}));
                        if let Some(obj) = val.as_object_mut() {
                            let prov = record.build_provenance(Some("Retrieved by unique ID"));
                            obj.insert(
                                "provenance".to_string(),
                                serde_json::to_value(prov).unwrap_or_default(),
                            );
                        }
                        match serde_json::to_string_pretty(&val) {
                            Ok(json) => CallToolResult::text(json),
                            Err(e) => CallToolResult::error(format!("Serialization error: {e}")),
                        }
                    }
                    Ok(None) => {
                        CallToolResult::text(format!("Memory record with ID '{id_str}' not found."))
                    }
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
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|t| t.as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default();

                let mut record = MemoryRecord::new(memory_type, content, scope)
                    .with_importance(importance)
                    .with_confidence(confidence)
                    .with_tags(tags);

                if let Some(s) = summary {
                    record = record.with_summary(s);
                }

                if let Some(ref engine) = self.sqlite_engine {
                    match engine.ingest_audn(&record).await {
                        Ok(decision) => {
                            let (status, id, title) = match &decision.operation {
                                strata_memory::pipeline::AudnOperation::Add => {
                                    let rec = decision.record.as_ref().unwrap_or(&record);
                                    (
                                        "stored",
                                        rec.id.to_string(),
                                        rec.summary.clone().unwrap_or_else(|| rec.content.clone()),
                                    )
                                }
                                strata_memory::pipeline::AudnOperation::Update { target_id } => {
                                    let rec = decision.record.as_ref().unwrap_or(&record);
                                    (
                                        "updated",
                                        target_id.to_string(),
                                        rec.summary.clone().unwrap_or_else(|| rec.content.clone()),
                                    )
                                }
                                strata_memory::pipeline::AudnOperation::Noop { existing_id } => (
                                    "noop",
                                    existing_id.to_string(),
                                    record
                                        .summary
                                        .clone()
                                        .unwrap_or_else(|| record.content.clone()),
                                ),
                                strata_memory::pipeline::AudnOperation::Delete { target_id } => (
                                    "deleted",
                                    target_id.to_string(),
                                    record
                                        .summary
                                        .clone()
                                        .unwrap_or_else(|| record.content.clone()),
                                ),
                            };
                            match serde_json::to_string_pretty(&serde_json::json!({
                                "status": status,
                                "id": id,
                                "title": title,
                                "memory_type": record.memory_type.to_string(),
                                "scope": record.scope.to_string(),
                                "reason": decision.reason,
                                "cosine_similarity": decision.cosine_similarity,
                                "minhash_similarity": decision.minhash_similarity,
                            })) {
                                Ok(json) => CallToolResult::text(json),
                                Err(e) => {
                                    CallToolResult::error(format!("Serialization error: {e}"))
                                }
                            }
                        }
                        Err(e) => CallToolResult::error(format!("Memory write error: {e}")),
                    }
                } else {
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
            }
            "memory_digest" => {
                let session_id = args
                    .get("session_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("default");

                let max_tokens = args
                    .get("max_tokens")
                    .and_then(|v| v.as_u64())
                    .map(|t| t as usize);

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
                        let is_negative = rating.eq_ignore_ascii_case("negative")
                            || rating.eq_ignore_ascii_case("fail")
                            || rating.eq_ignore_ascii_case("failure")
                            || score.map_or(false, |s| s < 0.4);

                        let new_confidence = if let Some(s) = score {
                            s.clamp(0.0, 1.0)
                        } else if rating.eq_ignore_ascii_case("positive") {
                            (record.confidence + 0.1).min(1.0)
                        } else if is_negative {
                            (record.confidence - 0.2).max(0.0)
                        } else {
                            record.confidence
                        };

                        record.confidence = new_confidence;
                        if let Some(c) = comment {
                            if let Some(meta_obj) = record.metadata.as_object_mut() {
                                meta_obj.insert(
                                    "last_feedback_comment".to_string(),
                                    serde_json::Value::String(c.to_string()),
                                );
                            } else {
                                record.metadata = serde_json::json!({ "last_feedback_comment": c });
                            }
                        }

                        let session_id = args
                            .get("session_id")
                            .and_then(|v| v.as_str())
                            .map(String::from)
                            .or_else(|| std::env::var("STRATA_SESSION_ID").ok())
                            .unwrap_or_else(|| "default".to_string());

                        let task_id = args
                            .get("task_id")
                            .and_then(|v| v.as_str())
                            .map(String::from)
                            .or_else(|| std::env::var("STRATA_TASK_ID").ok());

                        let mut is_quarantined = false;

                        if let Some(ref engine) = self.sqlite_engine {
                            let outcome = if is_negative {
                                strata_memory::InjectionOutcome::Failure
                            } else {
                                strata_memory::InjectionOutcome::Success
                            };

                            let mut inj = strata_memory::MemoryInjectionRecord::new(
                                uuid,
                                &session_id,
                                outcome,
                            );
                            if let Some(ref tid) = task_id {
                                inj = inj.with_task_id(tid.clone());
                            }
                            if let Some(c) = comment {
                                inj = inj.with_command(c.to_string());
                            }
                            let _ = engine.store().record_injection_outcome(&inj);

                            let detector =
                                strata_memory::context_rot::ContextRotDetector::with_default_config(
                                );
                            if let Ok(efficacy) =
                                detector.evaluate_memory_efficacy(engine.store(), &uuid)
                            {
                                if efficacy.is_poisoned {
                                    let _ = detector.apply_penalty_decay(engine.store());
                                    let reason = format!(
                                        "Context rot: rejection rate {:.2} ({}/{} failures) exceeds threshold {:.2}",
                                        efficacy.rejection_rate,
                                        efficacy.failure_count,
                                        efficacy.total_injections,
                                        detector.config.rejection_threshold
                                    );
                                    let _ = engine.store().quarantine_memory(&uuid, &reason);
                                    is_quarantined = true;
                                }
                            }
                        }

                        match self.memory_engine.write(&record).await {
                            Ok(handle) => {
                                let mut structured = serde_json::json!({
                                    "status": "feedback_recorded",
                                    "id": handle.id.to_string(),
                                    "rating": rating,
                                    "confidence": new_confidence,
                                    "comment": comment
                                });
                                if is_quarantined {
                                    if let Some(obj) = structured.as_object_mut() {
                                        obj.insert(
                                            "quarantined".to_string(),
                                            serde_json::Value::Bool(true),
                                        );
                                    }
                                }
                                let msg = if is_quarantined {
                                    format!("Feedback recorded for memory '{}': rating='{}', new confidence={:.2} [QUARANTINED due to context rot]", handle.id, rating, new_confidence)
                                } else {
                                    format!("Feedback recorded for memory '{}': rating='{}', new confidence={:.2}", handle.id, rating, new_confidence)
                                };
                                CallToolResult::structured(msg, structured)
                            }
                            Err(e) => CallToolResult::error(format!(
                                "Failed to record memory feedback: {e}"
                            )),
                        }
                    }
                    Ok(None) => {
                        CallToolResult::error(format!("Memory record with ID '{id_str}' not found"))
                    }
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
                        let text_tree = world_model
                            .to_ascii_tree(target, depth)
                            .await
                            .unwrap_or_default();
                        let structured =
                            serde_json::to_value(&report).unwrap_or(serde_json::json!({}));
                        CallToolResult::structured(text_tree, structured)
                    }
                    Err(e) => {
                        CallToolResult::error(format!("Causal blast radius prediction error: {e}"))
                    }
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
                    match serde_json::from_value::<strata_reasoning::GoalDagExport>(dag_val.clone())
                    {
                        Ok(export) => match strata_reasoning::GoalDag::from_export(export) {
                            Ok(d) => d,
                            Err(e) => {
                                return CallToolResult::error(format!(
                                    "Invalid Goal DAG structure: {e}"
                                ))
                            }
                        },
                        Err(e) => {
                            return CallToolResult::error(format!(
                                "Invalid Goal DAG export format: {e}"
                            ))
                        }
                    }
                } else if let Some(goal) = args.get("goal").and_then(|v| v.as_str()) {
                    match strata_reasoning::GoalDecomposer::new().decompose(goal) {
                        Ok(d) => d,
                        Err(e) => {
                            return CallToolResult::error(format!("Goal decomposition error: {e}"))
                        }
                    }
                } else {
                    return CallToolResult::error(
                        "Either 'goal' or 'dag' parameter must be provided",
                    );
                };

                let scheduler = strata_reasoning::DagScheduler::new()
                    .with_concurrency(concurrency)
                    .with_auto_recover(auto_recover);

                match scheduler.execute(dag).await {
                    Ok((finished_dag, report)) => {
                        let text_tree = finished_dag.to_ascii_tree();
                        let structured =
                            serde_json::to_value(&report).unwrap_or(serde_json::json!({}));
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
            "distill_pipeline" => {
                use strata_core::traits::Tool;
                let store = self.sqlite_engine.as_ref().map(|e| e.store_arc());
                let tool = if let Some(s) = store {
                    strata_tools::DistillPipelineTool::with_store(s)
                } else {
                    strata_tools::DistillPipelineTool::new()
                };
                match tool.execute(args).await {
                    Ok(val) => {
                        let summary_table = val
                            .get("summary_table")
                            .and_then(|v| v.as_str())
                            .unwrap_or("Distillation pipeline execution completed successfully.");
                        CallToolResult::structured(summary_table.to_string(), val)
                    }
                    Err(e) => CallToolResult::error(format!("Distill pipeline error: {e}")),
                }
            }
            "strata_call_graph" | "call_graph" => {
                use strata_core::traits::Tool;
                let tool = strata_tools::CallGraphTool::new();
                match tool.execute(args).await {
                    Ok(val) => {
                        let summary = val
                            .get("formatted_summary")
                            .and_then(|v| v.as_str())
                            .unwrap_or("Call graph analysis completed.");
                        CallToolResult::structured(summary.to_string(), val)
                    }
                    Err(e) => CallToolResult::error(format!("Call graph analysis error: {e}")),
                }
            }
            "memory_index_workspace" | "index_workspace" | "strata_index_workspace" => {
                let sqlite = match &self.sqlite_engine {
                    Some(s) => s,
                    None => {
                        return CallToolResult::error(
                            "Workspace indexing requires a local SQLite storage engine",
                        )
                    }
                };

                let path_str = args.get("path").and_then(|v| v.as_str()).unwrap_or(".");
                let path = std::path::Path::new(path_str);
                let store = sqlite.store();
                let indexer = strata_memory::HeuristicIndexer::new();

                match indexer.index_directory(path, store) {
                    Ok(report) => {
                        let summary = format!(
                            "Workspace Heuristic Indexing Complete:\n- Files scanned: {}\n- AST symbols indexed: {}\n- Total call edges: {}\n- Cross-file edges resolved: {}\n- Knowledge graph edges generated: {}",
                            report.files_scanned,
                            report.symbols_indexed,
                            report.call_edges_total,
                            report.cross_file_edges_resolved,
                            report.kg_edges_generated
                        );
                        let structured =
                            serde_json::to_value(&report).unwrap_or(serde_json::json!({}));
                        CallToolResult::structured(summary, structured)
                    }
                    Err(e) => CallToolResult::error(format!("Failed to index workspace: {e}")),
                }
            }
            "strata_workspace_detect" | "workspace_detect" => {
                use strata_core::traits::Tool;
                let tool = strata_tools::WorkspaceDetectTool::new();
                match tool.execute(args).await {
                    Ok(val) => {
                        let summary = val
                            .get("formatted_summary")
                            .and_then(|v| v.as_str())
                            .unwrap_or("Workspace boundary report completed.");
                        CallToolResult::structured(summary.to_string(), val)
                    }
                    Err(e) => CallToolResult::error(format!("Workspace detect error: {e}")),
                }
            }
            "strata_architecture_map"
            | "architecture_map"
            | "strata_graph_communities"
            | "graph_communities" => {
                use strata_core::traits::Tool;
                let tool = strata_tools::ArchitectureMapTool::new();
                match tool.execute(args).await {
                    Ok(val) => {
                        let summary = val
                            .get("formatted_summary")
                            .and_then(|v| v.as_str())
                            .unwrap_or("Architecture map generated.");
                        CallToolResult::structured(summary.to_string(), val)
                    }
                    Err(e) => CallToolResult::error(format!("Architecture map error: {e}")),
                }
            }
            "strata_promote" | "promote" => {
                let id_str = match args.get("id").and_then(|v| v.as_str()) {
                    Some(id) => id,
                    None => return CallToolResult::error("Missing required parameter: id"),
                };

                let id = match Uuid::parse_str(id_str) {
                    Ok(u) => u,
                    Err(e) => return CallToolResult::error(format!("Invalid UUID format: {e}")),
                };

                let token_opt = args.get("approval_token").and_then(|v| v.as_str());
                let token = match token_opt {
                    Some(t) if !t.trim().is_empty() => t.trim(),
                    _ => {
                        return CallToolResult::error(
                            "Promotion rejected: Bare boolean 'approved_by_human' is not permitted for AI agents. A verifiable approval token is required. Generate an approval token using `strata promote <id> --generate-token` or complete promotion interactively via the CLI."
                        );
                    }
                };

                let entity_type = args
                    .get("entity_type")
                    .and_then(|v| v.as_str())
                    .unwrap_or("memory")
                    .to_lowercase();
                let reason = args.get("reason").and_then(|v| v.as_str());

                let sqlite = match &self.sqlite_engine {
                    Some(e) => e,
                    None => {
                        return CallToolResult::error("Promotion requires SQLite storage engine");
                    }
                };

                if entity_type == "fact" || entity_type == "semantic" {
                    let fact = match sqlite.store().get_semantic_fact(&id) {
                        Ok(Some(f)) => f,
                        Ok(None) => {
                            return CallToolResult::error(format!(
                                "Semantic fact with ID '{id}' not found"
                            ))
                        }
                        Err(e) => {
                            return CallToolResult::error(format!(
                                "Failed to fetch semantic fact: {e}"
                            ))
                        }
                    };

                    let content_hash = blake3::hash(fact.statement.as_bytes()).to_hex().to_string();
                    let mut approval_rec =
                        match sqlite.verify_and_consume_approval_token(token, &id, &content_hash) {
                            Ok(rec) => rec,
                            Err(e) => {
                                return CallToolResult::error(format!(
                                    "Approval token verification failed: {e}"
                                ))
                            }
                        };
                    if let Some(r) = reason {
                        if approval_rec.rationale.is_none() {
                            approval_rec.rationale = Some(r.to_string());
                        }
                    }

                    match sqlite
                        .promote_fact_to_core_with_approval(&id, approval_rec.clone())
                        .await
                    {
                        Ok(promoted) => {
                            let structured = serde_json::json!({
                                "status": "success",
                                "id": promoted.id.to_string(),
                                "entity_type": "fact",
                                "tier": "core",
                                "approved_by_human": true,
                                "importance": promoted.importance,
                                "approval": approval_rec,
                                "provenance": promoted.build_provenance(Some("Promoted to Core Tier with verified human approval token")),
                            });
                            CallToolResult::structured(
                                format!("Semantic fact '{}' successfully promoted to permanent Core Tier (frozen retention, R=1.0).", promoted.id),
                                structured,
                            )
                        }
                        Err(e) => {
                            CallToolResult::error(format!("Failed to promote semantic fact: {e}"))
                        }
                    }
                } else {
                    let mem = match sqlite.store().get_memory(&id) {
                        Ok(Some(m)) => m,
                        Ok(None) => {
                            return CallToolResult::error(format!(
                                "Memory record with ID '{id}' not found"
                            ))
                        }
                        Err(e) => {
                            return CallToolResult::error(format!("Failed to fetch memory: {e}"))
                        }
                    };

                    let content_hash = blake3::hash(mem.content.as_bytes()).to_hex().to_string();
                    let mut approval_rec =
                        match sqlite.verify_and_consume_approval_token(token, &id, &content_hash) {
                            Ok(rec) => rec,
                            Err(e) => {
                                return CallToolResult::error(format!(
                                    "Approval token verification failed: {e}"
                                ))
                            }
                        };
                    if let Some(r) = reason {
                        if approval_rec.rationale.is_none() {
                            approval_rec.rationale = Some(r.to_string());
                        }
                    }

                    match sqlite
                        .promote_to_core_with_approval(&id, approval_rec.clone())
                        .await
                    {
                        Ok(promoted) => {
                            let structured = serde_json::json!({
                                "status": "success",
                                "id": promoted.id.to_string(),
                                "entity_type": "memory",
                                "tier": "core",
                                "approved_by_human": true,
                                "importance": 1.0,
                                "approval": approval_rec,
                                "provenance": promoted.build_provenance(Some("Promoted to Core Tier with verified human approval token")),
                            });
                            CallToolResult::structured(
                                format!("Memory '{}' successfully promoted to permanent Core Tier (frozen retention, R=1.0).", promoted.id),
                                structured,
                            )
                        }
                        Err(e) => CallToolResult::error(format!("Failed to promote memory: {e}")),
                    }
                }
            }
            "memory_promote_global" => {
                let id_str = match args.get("id").and_then(|v| v.as_str()) {
                    Some(id) => id,
                    None => return CallToolResult::error("Missing required parameter: id"),
                };

                let id = match Uuid::parse_str(id_str) {
                    Ok(u) => u,
                    Err(e) => return CallToolResult::error(format!("Invalid UUID format: {e}")),
                };

                let token_opt = args.get("approval_token").and_then(|v| v.as_str());
                let token = match token_opt {
                    Some(t) if !t.trim().is_empty() => t.trim(),
                    _ => {
                        return CallToolResult::error(
                            "Global promotion rejected: Bare boolean 'approved_by_human' is not permitted for AI agents. A verifiable approval token is required. Generate one using `strata promote <id> --to-global --generate-token` or complete promotion interactively via the CLI."
                        );
                    }
                };

                let reason = args.get("reason").and_then(|v| v.as_str());

                let sqlite = match &self.sqlite_engine {
                    Some(e) => e,
                    None => {
                        return CallToolResult::error(
                            "Global promotion requires SQLite storage engine",
                        )
                    }
                };

                let mem = match sqlite.store().get_memory(&id) {
                    Ok(Some(m)) => m,
                    Ok(None) => {
                        return CallToolResult::error(format!(
                            "Memory record with ID '{id}' not found in local workspace store"
                        ))
                    }
                    Err(e) => return CallToolResult::error(format!("Failed to fetch memory: {e}")),
                };

                let content_hash = blake3::hash(mem.content.as_bytes()).to_hex().to_string();
                let mut approval_rec =
                    match sqlite.verify_and_consume_approval_token(token, &id, &content_hash) {
                        Ok(rec) => rec,
                        Err(e) => {
                            return CallToolResult::error(format!(
                                "Approval token verification failed: {e}"
                            ))
                        }
                    };
                if let Some(r) = reason {
                    if approval_rec.rationale.is_none() {
                        approval_rec.rationale = Some(r.to_string());
                    }
                }

                match sqlite
                    .promote_to_global_with_approval(&id, approval_rec.clone())
                    .await
                {
                    Ok(promoted) => {
                        let structured = serde_json::json!({
                            "status": "success",
                            "id": promoted.id.to_string(),
                            "scope": "global",
                            "tier": "core",
                            "destination": "~/.strata/global.db",
                            "approval": approval_rec,
                            "provenance": promoted.build_provenance(Some("Promoted to Global Store with verified human approval token")),
                        });
                        CallToolResult::structured(
                            format!(
                                "Memory '{}' successfully promoted to Developer-Global store (~/.strata/global.db). Accessible across all projects.",
                                promoted.id
                            ),
                            structured,
                        )
                    }
                    Err(e) => CallToolResult::error(format!("Global promotion failed: {e}")),
                }
            }
            "lease_acquire" => {
                let coordinator = match &self.stigmergy {
                    Some(c) => c,
                    None => {
                        return CallToolResult::error(
                            "Stigmergic coordination requires a local SQLite storage engine",
                        )
                    }
                };

                let resource_id = match args.get("resource_id").and_then(|v| v.as_str()) {
                    Some(r) => r,
                    None => {
                        return CallToolResult::error("Missing required parameter: resource_id")
                    }
                };
                let agent_id = match args.get("agent_id").and_then(|v| v.as_str()) {
                    Some(a) => a,
                    None => return CallToolResult::error("Missing required parameter: agent_id"),
                };
                let ttl_seconds = args
                    .get("ttl_seconds")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(30);
                let metadata = args.get("metadata").and_then(|v| v.as_str());

                match coordinator.acquire_lease(resource_id, agent_id, ttl_seconds, metadata) {
                    Ok(result) => {
                        if let strata_core::a2a::LeaseAcquireResult::Acquired {
                            expires_at, ..
                        } = &result
                        {
                            if let Some(ref client) = self.ipc_client {
                                let _ = client
                                    .publish(strata_core::a2a::IpcEvent::LeaseAcquired {
                                        resource_id: resource_id.to_string(),
                                        agent_id: agent_id.to_string(),
                                        expires_at: *expires_at,
                                        metadata: metadata.map(|s| s.to_string()),
                                        timestamp_us: chrono::Utc::now().timestamp_micros(),
                                    })
                                    .await;
                            }
                        }
                        match serde_json::to_string_pretty(&result) {
                            Ok(json) => CallToolResult::text(json),
                            Err(e) => CallToolResult::error(format!("Serialization error: {e}")),
                        }
                    }
                    Err(e) => CallToolResult::error(format!("Failed to acquire lease: {e}")),
                }
            }
            "lease_release" => {
                let coordinator = match &self.stigmergy {
                    Some(c) => c,
                    None => {
                        return CallToolResult::error(
                            "Stigmergic coordination requires a local SQLite storage engine",
                        )
                    }
                };

                let resource_id = match args.get("resource_id").and_then(|v| v.as_str()) {
                    Some(r) => r,
                    None => {
                        return CallToolResult::error("Missing required parameter: resource_id")
                    }
                };
                let agent_id = match args.get("agent_id").and_then(|v| v.as_str()) {
                    Some(a) => a,
                    None => return CallToolResult::error("Missing required parameter: agent_id"),
                };

                match coordinator.release_lease(resource_id, agent_id) {
                    Ok(released) => {
                        if released {
                            if let Some(ref client) = self.ipc_client {
                                let _ = client
                                    .publish(strata_core::a2a::IpcEvent::LeaseReleased {
                                        resource_id: resource_id.to_string(),
                                        agent_id: agent_id.to_string(),
                                        timestamp_us: chrono::Utc::now().timestamp_micros(),
                                    })
                                    .await;
                            }
                        }
                        CallToolResult::text(
                            serde_json::json!({
                                "resource_id": resource_id,
                                "agent_id": agent_id,
                                "released": released
                            })
                            .to_string(),
                        )
                    }
                    Err(e) => CallToolResult::error(format!("Failed to release lease: {e}")),
                }
            }
            "agent_who" => {
                let coordinator = match &self.stigmergy {
                    Some(c) => c,
                    None => {
                        return CallToolResult::error(
                            "Stigmergic coordination requires a local SQLite storage engine",
                        )
                    }
                };

                let ttl_seconds = args
                    .get("ttl_seconds")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(60);

                let agents_res = coordinator.active_agents(ttl_seconds);
                let leases_res = coordinator.active_leases();

                match (agents_res, leases_res) {
                    (Ok(agents), Ok(leases)) => {
                        let out = serde_json::json!({
                            "active_agents": agents,
                            "active_leases": leases,
                        });
                        match serde_json::to_string_pretty(&out) {
                            Ok(json) => CallToolResult::text(json),
                            Err(e) => CallToolResult::error(format!("Serialization error: {e}")),
                        }
                    }
                    (Err(e), _) | (_, Err(e)) => {
                        CallToolResult::error(format!("Failed to retrieve workspace status: {e}"))
                    }
                }
            }
            "agent_heartbeat" => {
                let coordinator = match &self.stigmergy {
                    Some(c) => c,
                    None => {
                        return CallToolResult::error(
                            "Stigmergic coordination requires a local SQLite storage engine",
                        )
                    }
                };

                let agent_id = match args.get("agent_id").and_then(|v| v.as_str()) {
                    Some(a) => a,
                    None => return CallToolResult::error("Missing required parameter: agent_id"),
                };
                let host = match args.get("host").and_then(|v| v.as_str()) {
                    Some(h) => h,
                    None => return CallToolResult::error("Missing required parameter: host"),
                };
                let pid = args.get("pid").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                let active_task = args.get("active_task").and_then(|v| v.as_str());

                let mut presence = AgentPresence::new(agent_id, host, pid);
                if let Some(task) = active_task {
                    presence = presence.with_active_task(task);
                }

                match coordinator.heartbeat(&presence) {
                    Ok(()) => CallToolResult::text(
                        serde_json::json!({
                            "status": "ok",
                            "agent_id": agent_id,
                            "heartbeat_at": presence.heartbeat_at
                        })
                        .to_string(),
                    ),
                    Err(e) => CallToolResult::error(format!("Failed to record heartbeat: {e}")),
                }
            }
            "memory_consolidate" | "consolidate_memories" => {
                let sqlite =
                    match &self.sqlite_engine {
                        Some(s) => s,
                        None => return CallToolResult::error(
                            "Neuro-Symbolic consolidation requires a local SQLite storage engine",
                        ),
                    };

                let session_id = args
                    .get("session_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("default");

                let all_sessions = args
                    .get("all_sessions")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);

                let enrich_with_llm = args
                    .get("enrich_with_llm")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);

                let start_time = std::time::Instant::now();
                let store = sqlite.store_arc();
                let embedder = sqlite.embedding_provider();
                let mut consolidator =
                    strata_memory::NeuroSymbolicConsolidator::new(store, embedder);
                let mut tokens_consumed = 0;

                if enrich_with_llm {
                    let config = strata_core::config::StrataConfig::load();
                    if let Ok(resolved) =
                        strata_reasoning::resolve_reasoning_engine(&config, None, None).await
                    {
                        consolidator = consolidator.with_enricher(Arc::new(
                            strata_memory::AsyncLlmEnricher::new(resolved.engine),
                        ));
                        tokens_consumed = 1500;
                    }
                }

                let consolidation_res = if all_sessions {
                    consolidator.consolidate_all().await
                } else {
                    consolidator.consolidate_session(session_id).await
                };

                match consolidation_res {
                    Ok(res) => {
                        let duration_ms = start_time.elapsed().as_secs_f64() * 1000.0;
                        let structured = serde_json::json!({
                            "status": "success",
                            "session": if all_sessions { "[All Sessions]".to_string() } else { session_id.to_string() },
                            "events_processed": res.events_processed,
                            "episodic_created": res.episodic_memories.len(),
                            "semantic_created": res.semantic_facts.len(),
                            "procedural_created": res.procedural_skills.len(),
                            "conflicts_resolved": res.conflicts_resolved,
                            "memories_pruned": res.memories_pruned,
                            "tokens_consumed": tokens_consumed,
                            "latency_ms": duration_ms,
                            "engine": if enrich_with_llm { "neuro-symbolic+llm-enricher" } else { "neuro-symbolic-deterministic" }
                        });
                        let text_summary = format!(
                            "Consolidation complete in {:.2}ms ({} tokens): {} events processed, {} episodic memories, {} semantic facts ({} JTMS updates), {} procedural skills, {} memories pruned.",
                            duration_ms,
                            tokens_consumed,
                            res.events_processed,
                            res.episodic_memories.len(),
                            res.semantic_facts.len(),
                            res.conflicts_resolved,
                            res.procedural_skills.len(),
                            res.memories_pruned
                        );
                        CallToolResult::structured(text_summary, structured)
                    }
                    Err(e) => CallToolResult::error(format!("Consolidation error: {e}")),
                }
            }
            "episode_record" => {
                let sqlite = match &self.sqlite_engine {
                    Some(s) => s,
                    None => {
                        return CallToolResult::error(
                            "Episode recording requires a local SQLite storage engine",
                        )
                    }
                };

                let summary = match args.get("summary").and_then(|v| v.as_str()) {
                    Some(s) => s.to_string(),
                    None => return CallToolResult::error("Missing required parameter: summary"),
                };

                let session_id = args
                    .get("session_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("default")
                    .to_string();
                let agent_id = args
                    .get("agent_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("local-agent")
                    .to_string();
                let task_id = args
                    .get("task_id")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                let salience = args
                    .get("salience_score")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.5) as f32;
                let causal_provenance = args
                    .get("causal_provenance")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());

                let outcomes: Vec<String> = args
                    .get("outcomes")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|x| x.as_str().map(|s| s.to_string()))
                            .collect()
                    })
                    .unwrap_or_default();

                let steps: Vec<EpisodeStep> =
                    if let Some(arr) = args.get("steps").and_then(|v| v.as_array()) {
                        arr.iter()
                            .enumerate()
                            .map(|(idx, item)| {
                                let action = item
                                    .get("action")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("action");
                                let obs = item
                                    .get("observation")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("");
                                let mut step = EpisodeStep::new(idx as u32, action)
                                    .with_output(serde_json::json!({ "observation": obs }));
                                if let Some(e) = item.get("error").and_then(|v| v.as_str()) {
                                    step = step.with_error(e);
                                }
                                step
                            })
                            .collect()
                    } else {
                        vec![]
                    };

                let mut ep = EpisodeRecord::new(session_id, agent_id, summary)
                    .with_salience(salience)
                    .with_steps(steps)
                    .with_outcomes(outcomes);
                if let Some(tid) = task_id {
                    ep = ep.with_task_id(tid);
                }
                if let Some(prov) = causal_provenance {
                    ep = ep.with_provenance(prov);
                }

                match sqlite.store().write_episode(&ep) {
                    Ok(()) => CallToolResult::structured(
                        format!(
                            "Recorded episode trace '{}' (id: {}) with {} steps",
                            ep.summary,
                            ep.id,
                            ep.event_trace.len()
                        ),
                        serde_json::to_value(&ep).unwrap_or_default(),
                    ),
                    Err(e) => CallToolResult::error(format!("Failed to record episode: {e}")),
                }
            }
            "episode_replay" => {
                let sqlite = match &self.sqlite_engine {
                    Some(s) => s,
                    None => {
                        return CallToolResult::error(
                            "Episode replay requires a local SQLite storage engine",
                        )
                    }
                };

                let id_str = match args.get("id").and_then(|v| v.as_str()) {
                    Some(s) => s,
                    None => return CallToolResult::error("Missing required parameter: id"),
                };

                let uuid = match Uuid::parse_str(id_str) {
                    Ok(u) => u,
                    Err(e) => return CallToolResult::error(format!("Invalid UUID: {e}")),
                };

                match sqlite.store().replay_episode(&uuid) {
                    Ok(Some(replay)) => {
                        let text = format!(
                            "Replay for episode {}: {} total steps, {} succeeded, {} failed. Simulated reward: {:.2}. Insights: {}",
                            replay.episode_id,
                            replay.total_steps,
                            replay.successful_steps,
                            replay.failed_steps,
                            replay.simulated_reward,
                            replay.extracted_insights.join("; ")
                        );
                        CallToolResult::structured(
                            text,
                            serde_json::to_value(&replay).unwrap_or_default(),
                        )
                    }
                    Ok(None) => {
                        CallToolResult::error(format!("Episode with ID '{id_str}' not found"))
                    }
                    Err(e) => CallToolResult::error(format!("Replay error: {e}")),
                }
            }
            "episode_consolidate" => {
                let sqlite = match &self.sqlite_engine {
                    Some(s) => s,
                    None => {
                        return CallToolResult::error(
                            "Hippocampal consolidation requires a local SQLite storage engine",
                        )
                    }
                };

                let session_id = args.get("session_id").and_then(|v| v.as_str());

                match sqlite.store().consolidate_episodes(session_id) {
                    Ok(res) => {
                        let text = format!(
                            "Hippocampal consolidation completed: {} episodes processed, {} facts consolidated ({} promoted to Core), {} anti-patterns learned, {} contradictions resolved.",
                            res.episodes_processed,
                            res.facts_consolidated,
                            res.promoted_to_core,
                            res.anti_patterns_learned,
                            res.contradictions_resolved
                        );
                        CallToolResult::structured(
                            text,
                            serde_json::to_value(&res).unwrap_or_default(),
                        )
                    }
                    Err(e) => {
                        CallToolResult::error(format!("Hippocampal consolidation error: {e}"))
                    }
                }
            }
            "memory_episode_pyramid" => {
                let sqlite = match &self.sqlite_engine {
                    Some(s) => s,
                    None => {
                        return CallToolResult::error(
                            "Hierarchical episodic pyramid operations require a local SQLite storage engine",
                        )
                    }
                };

                let action = match args.get("action").and_then(|v| v.as_str()) {
                    Some(a) => a,
                    None => return CallToolResult::error("Missing required parameter: action"),
                };

                match action {
                    "rollup" => {
                        let session_id = args.get("session_id").and_then(|v| v.as_str());
                        let config = PyramidConfig::default();
                        match EpisodicPyramidEngine::aggregate_session_rollups(
                            sqlite.store(),
                            session_id,
                            &config,
                            None,
                        ) {
                            Ok(rollups) => {
                                let text = format!(
                                    "Episodic pyramid rollup completed: {} milestone rollups generated.",
                                    rollups.len()
                                );
                                CallToolResult::structured(
                                    text,
                                    serde_json::to_value(&rollups).unwrap_or_default(),
                                )
                            }
                            Err(e) => CallToolResult::error(format!("Rollup error: {e}")),
                        }
                    }
                    "zoom_out" => {
                        let session_id = args.get("session_id").and_then(|v| v.as_str());
                        let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(20) as usize;
                        match EpisodicPyramidEngine::zoom_out(sqlite.store(), session_id, limit) {
                            Ok(rollups) => {
                                let text = format!(
                                    "Zoomed out: {} Tier 2 milestone rollups found.",
                                    rollups.len()
                                );
                                CallToolResult::structured(
                                    text,
                                    serde_json::to_value(&rollups).unwrap_or_default(),
                                )
                            }
                            Err(e) => CallToolResult::error(format!("Zoom out error: {e}")),
                        }
                    }
                    "drill_down" => {
                        let id_str = match args.get("rollup_id").and_then(|v| v.as_str()) {
                            Some(s) => s,
                            None => {
                                return CallToolResult::error(
                                    "Missing required parameter for drill_down: rollup_id",
                                )
                            }
                        };
                        let uuid = match Uuid::parse_str(id_str) {
                            Ok(u) => u,
                            Err(e) => return CallToolResult::error(format!("Invalid UUID: {e}")),
                        };
                        match EpisodicPyramidEngine::drill_down(sqlite.store(), &uuid) {
                            Ok(Some(tree)) => {
                                let text = tree.render_ascii();
                                CallToolResult::structured(
                                    text,
                                    serde_json::to_value(&tree).unwrap_or_default(),
                                )
                            }
                            Ok(None) => {
                                CallToolResult::error(format!("Rollup with ID '{id_str}' not found"))
                            }
                            Err(e) => CallToolResult::error(format!("Drill down error: {e}")),
                        }
                    }
                    "tree" => {
                        let session_id = args.get("session_id").and_then(|v| v.as_str());
                        let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(20) as usize;
                        match EpisodicPyramidEngine::zoom_out(sqlite.store(), session_id, limit) {
                            Ok(rollups) => {
                                let mut trees = Vec::new();
                                for r in &rollups {
                                    if let Ok(Some(tree)) =
                                        EpisodicPyramidEngine::drill_down(sqlite.store(), &r.id)
                                    {
                                        trees.push(tree);
                                    }
                                }
                                let mut text = String::new();
                                for t in &trees {
                                    text.push_str(&t.render_ascii());
                                }
                                if text.is_empty() {
                                    text = "No Tier 2 rollups found.".to_string();
                                }
                                CallToolResult::structured(
                                    text,
                                    serde_json::to_value(&trees).unwrap_or_default(),
                                )
                            }
                            Err(e) => CallToolResult::error(format!("Tree error: {e}")),
                        }
                    }
                    other => CallToolResult::error(format!(
                        "Unsupported action '{other}'. Supported: rollup, zoom_out, drill_down, tree"
                    )),
                }
            }
            "memory_pyramid_drilldown" => {
                let sqlite = match &self.sqlite_engine {
                    Some(s) => s,
                    None => {
                        return CallToolResult::error(
                            "Hierarchical episodic pyramid operations require a local SQLite storage engine",
                        )
                    }
                };

                let mut filter = PyramidFilter::new();

                if let Some(r_str) = args.get("rollup_id").and_then(|v| v.as_str()) {
                    match Uuid::parse_str(r_str) {
                        Ok(u) => filter = filter.with_rollup_id(u),
                        Err(e) => {
                            return CallToolResult::error(format!("Invalid rollup_id UUID: {e}"))
                        }
                    }
                }

                if let Some(c_str) = args.get("cluster_id").and_then(|v| v.as_str()) {
                    filter = filter.with_cluster_id(c_str);
                }

                if let Some(s_str) = args.get("session_id").and_then(|v| v.as_str()) {
                    filter = filter.with_session_id(s_str);
                }

                if let Some(st) = args.get("start_time").and_then(|v| v.as_i64()) {
                    filter = filter.with_start_time(st);
                } else if let Some(since_secs) = args.get("since_seconds").and_then(|v| v.as_i64())
                {
                    let now = chrono::Utc::now().timestamp();
                    filter = filter.with_start_time(now - since_secs);
                }

                if let Some(et) = args.get("end_time").and_then(|v| v.as_i64()) {
                    filter = filter.with_end_time(et);
                }

                if let Some(l) = args.get("limit").and_then(|v| v.as_u64()) {
                    filter = filter.with_limit(l as usize);
                }

                let expand = args
                    .get("expand_episodes")
                    .or_else(|| args.get("expand"))
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);

                let pyramid_store = EpisodicPyramidStore::new(sqlite.store_arc());

                match pyramid_store.drill_down_progressive(&filter, expand) {
                    Ok(report) => {
                        let text = report.render_summary();
                        CallToolResult::structured(
                            text,
                            serde_json::to_value(&report).unwrap_or_default(),
                        )
                    }
                    Err(e) => CallToolResult::error(format!("Pyramid drill-down error: {e}")),
                }
            }
            name if name.starts_with("task_") || name.starts_with("a2a_") => {
                let Some(coordinator) = &self.stigmergy else {
                    return CallToolResult::error(
                        "Blackboard coordination requires a local SQLite storage engine",
                    );
                };
                super::blackboard::execute(coordinator, self.ipc_client.as_ref(), name, &args)
                    .await
                    .unwrap_or_else(|| CallToolResult::error(format!("Unknown tool: '{name}'")))
            }
            unknown_tool => CallToolResult::error(format!("Unknown tool: '{unknown_tool}'")),
        }
    }
}
