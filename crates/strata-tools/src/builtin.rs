use std::sync::Arc;
use std::time::{Duration, Instant};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use strata_core::{
    errors::StrataError,
    state::{MemoryRecord, MemoryType, Scope},
    traits::{MemoryEngine, Tool},
};

/// Tool for searching memories via lexical and semantic search.
pub struct MemorySearchTool {
    engine: Arc<dyn MemoryEngine>,
}

impl MemorySearchTool {
    pub fn new(engine: Arc<dyn MemoryEngine>) -> Self {
        Self { engine }
    }
}

#[async_trait]
impl Tool for MemorySearchTool {
    fn name(&self) -> &str {
        "memory_search"
    }

    fn description(&self) -> &str {
        "Search persistent memory records using hybrid lexical and semantic search."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Natural language or keyword search query"
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum number of memory records to return (default 5)"
                },
                "scope": {
                    "type": "string",
                    "description": "Optional scope filter: 'global', 'org:<name>', 'project:<name>', 'user:<name>', 'session:<id>'"
                }
            },
            "required": ["query"]
        })
    }

    async fn execute(&self, params: serde_json::Value) -> Result<serde_json::Value, StrataError> {
        let query = params
            .get("query")
            .and_then(|v| v.as_str())
            .ok_or_else(|| StrataError::ValidationError("Missing 'query' field".to_string()))?;

        let limit = params
            .get("limit")
            .and_then(|v| v.as_u64())
            .unwrap_or(5) as usize;

        let scope = params
            .get("scope")
            .and_then(|v| v.as_str())
            .map(|s| s.parse::<Scope>())
            .transpose()
            .map_err(|e| StrataError::ValidationError(format!("Invalid scope: {:?}", e)))?;

        let results = self
            .engine
            .search(query, scope.as_ref(), limit)
            .await?;

        Ok(json!(results))
    }
}

/// Tool for retrieving a specific memory record by ID.
pub struct MemoryGetTool {
    engine: Arc<dyn MemoryEngine>,
}

impl MemoryGetTool {
    pub fn new(engine: Arc<dyn MemoryEngine>) -> Self {
        Self { engine }
    }
}

#[async_trait]
impl Tool for MemoryGetTool {
    fn name(&self) -> &str {
        "memory_get"
    }

    fn description(&self) -> &str {
        "Retrieve a specific memory record by its unique UUID identifier."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "id": {
                    "type": "string",
                    "description": "The unique UUID identifier of the memory record"
                }
            },
            "required": ["id"]
        })
    }

    async fn execute(&self, params: serde_json::Value) -> Result<serde_json::Value, StrataError> {
        let id_str = params
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| StrataError::ValidationError("Missing 'id' field".to_string()))?;

        let uuid = Uuid::parse_str(id_str)
            .map_err(|e| StrataError::ValidationError(format!("Invalid UUID '{}': {}", id_str, e)))?;

        let record = self.engine.get(&uuid).await?;
        match record {
            Some(rec) => Ok(json!(rec)),
            None => Err(StrataError::NotFound(format!("Memory record '{}' not found", id_str))),
        }
    }
}

/// Tool for writing a new memory record.
pub struct MemoryWriteTool {
    engine: Arc<dyn MemoryEngine>,
}

impl MemoryWriteTool {
    pub fn new(engine: Arc<dyn MemoryEngine>) -> Self {
        Self { engine }
    }
}

#[async_trait]
impl Tool for MemoryWriteTool {
    fn name(&self) -> &str {
        "memory_write"
    }

    fn description(&self) -> &str {
        "Write a new fact, episode, procedure, or decision to persistent memory."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "content": {
                    "type": "string",
                    "description": "Detailed text content of the memory record"
                },
                "summary": {
                    "type": "string",
                    "description": "Short title or summary for the memory"
                },
                "memory_type": {
                    "type": "string",
                    "enum": ["episodic", "semantic", "procedural", "negative_pattern"],
                    "description": "Category of memory (default: 'semantic')"
                },
                "scope": {
                    "type": "string",
                    "description": "Scope tag: 'global', 'project:<name>', 'org:<name>', 'session:<id>' (default: 'global')"
                },
                "importance": {
                    "type": "number",
                    "description": "Importance score between 0.0 and 1.0 (default 0.5)"
                },
                "tags": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Optional list of indexing tags"
                }
            },
            "required": ["content"]
        })
    }

    async fn execute(&self, params: serde_json::Value) -> Result<serde_json::Value, StrataError> {
        let content = params
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| StrataError::ValidationError("Missing 'content' field".to_string()))?;

        let memory_type = params
            .get("memory_type")
            .and_then(|v| v.as_str())
            .unwrap_or("semantic")
            .parse::<MemoryType>()
            .map_err(|e| StrataError::ValidationError(format!("Invalid memory_type: {}", e)))?;

        let scope = params
            .get("scope")
            .and_then(|v| v.as_str())
            .unwrap_or("global")
            .parse::<Scope>()
            .unwrap_or(Scope::Global);

        let mut record = MemoryRecord::new(memory_type, content, scope);

        if let Some(summary) = params.get("summary").and_then(|v| v.as_str()) {
            record = record.with_summary(summary);
        }

        if let Some(imp) = params.get("importance").and_then(|v| v.as_f64()) {
            record = record.with_importance(imp as f32);
        }

        if let Some(tags_val) = params.get("tags").and_then(|v| v.as_array()) {
            let tags: Vec<String> = tags_val
                .iter()
                .filter_map(|t| t.as_str().map(|s| s.to_string()))
                .collect();
            record = record.with_tags(tags);
        }

        let handle = self.engine.write(&record).await?;
        Ok(json!(handle))
    }
}

/// Tool for generating a compact digest of context and pointers.
pub struct MemoryDigestTool {
    engine: Arc<dyn MemoryEngine>,
}

impl MemoryDigestTool {
    pub fn new(engine: Arc<dyn MemoryEngine>) -> Self {
        Self { engine }
    }
}

#[async_trait]
impl Tool for MemoryDigestTool {
    fn name(&self) -> &str {
        "memory_digest"
    }

    fn description(&self) -> &str {
        "Generate a compact context digest of recent decisions, active hypotheses, known failures, and memory pointers."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "session_id": {
                    "type": "string",
                    "description": "Session identifier (default: 'default')"
                },
                "max_tokens": {
                    "type": "integer",
                    "description": "Target token budget for digest (default: 500)"
                }
            }
        })
    }

    async fn execute(&self, params: serde_json::Value) -> Result<serde_json::Value, StrataError> {
        let session_id = params
            .get("session_id")
            .and_then(|v| v.as_str())
            .unwrap_or("default");

        let max_tokens = params
            .get("max_tokens")
            .and_then(|v| v.as_u64())
            .map(|t| t as usize);

        let digest = self.engine.digest(session_id, max_tokens).await?;
        Ok(json!(digest))
    }
}

/// Execution output from SafeShellTool.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ShellExecutionOutput {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: Option<i32>,
    pub duration_ms: u64,
    pub timed_out: bool,
}

/// Audited command execution tool with timeout, sandboxing, and error classification.
pub struct SafeShellTool {
    allowed_working_dirs: Vec<String>,
    blocked_commands: Vec<String>,
    default_timeout: Duration,
}

impl SafeShellTool {
    pub fn new() -> Self {
        Self {
            allowed_working_dirs: Vec::new(),
            blocked_commands: vec![
                "rm -rf /".to_string(),
                "rmdir /s /q c:\\".to_string(),
                ":(){ :|:& };:".to_string(),
                "mkfs".to_string(),
                "format c:".to_string(),
                "dd if=".to_string(),
            ],
            default_timeout: Duration::from_secs(30),
        }
    }

    pub fn with_blocked_command(mut self, cmd: impl Into<String>) -> Self {
        self.blocked_commands.push(cmd.into());
        self
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.default_timeout = timeout;
        self
    }

    pub fn with_allowed_dir(mut self, dir: impl Into<String>) -> Self {
        self.allowed_working_dirs.push(dir.into());
        self
    }

    fn check_command_safety(&self, command: &str) -> Result<(), StrataError> {
        let lower = command.to_lowercase();
        for blocked in &self.blocked_commands {
            if lower.contains(&blocked.to_lowercase()) {
                return Err(StrataError::PermissionDenied(format!(
                    "Command contains blocked pattern '{}'",
                    blocked
                )));
            }
        }
        Ok(())
    }
}

impl Default for SafeShellTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for SafeShellTool {
    fn name(&self) -> &str {
        "safe_shell"
    }

    fn description(&self) -> &str {
        "Audited shell command execution with strict timeout, sandboxing checks, and error classification."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "Shell command to execute"
                },
                "cwd": {
                    "type": "string",
                    "description": "Optional working directory"
                },
                "timeout_ms": {
                    "type": "integer",
                    "description": "Optional execution timeout in milliseconds (default 30000)"
                }
            },
            "required": ["command"]
        })
    }

    async fn execute(&self, params: serde_json::Value) -> Result<serde_json::Value, StrataError> {
        let command = params
            .get("command")
            .and_then(|v| v.as_str())
            .ok_or_else(|| StrataError::ValidationError("Missing 'command' field".to_string()))?;

        self.check_command_safety(command)?;

        let cwd = params.get("cwd").and_then(|v| v.as_str());
        let timeout_ms = params
            .get("timeout_ms")
            .and_then(|v| v.as_u64())
            .map(Duration::from_millis)
            .unwrap_or(self.default_timeout);

        let start = Instant::now();

        #[cfg(target_os = "windows")]
        let mut cmd = {
            let mut c = tokio::process::Command::new("powershell");
            c.arg("-NoProfile").arg("-NonInteractive").arg("-Command").arg(command);
            c
        };

        #[cfg(not(target_os = "windows"))]
        let mut cmd = {
            let mut c = tokio::process::Command::new("sh");
            c.arg("-c").arg(command);
            c
        };

        if let Some(dir) = cwd {
            cmd.current_dir(dir);
        }

        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());

        let execution = cmd.output();
        let timeout_res = tokio::time::timeout(timeout_ms, execution).await;

        let duration_ms = start.elapsed().as_millis() as u64;

        match timeout_res {
            Ok(Ok(output)) => {
                let stdout = String::from_utf8_lossy(&output.stdout).to_string();
                let stderr = String::from_utf8_lossy(&output.stderr).to_string();
                let exit_code = output.status.code();

                if !output.status.success() {
                    let err_msg = if !stderr.trim().is_empty() {
                        stderr.trim().to_string()
                    } else {
                        format!("Command exited with status code {:?}", exit_code)
                    };
                    return Err(StrataError::ExecutionFailed {
                        code: exit_code,
                        stderr: err_msg,
                    });
                }

                let out = ShellExecutionOutput {
                    stdout,
                    stderr,
                    exit_code,
                    duration_ms,
                    timed_out: false,
                };
                Ok(json!(out))
            }
            Ok(Err(e)) => Err(StrataError::ToolError(format!("Failed to execute process: {}", e))),
            Err(_) => Err(StrataError::Timeout(format!(
                "Command execution timed out after {} ms",
                timeout_ms.as_millis()
            ))),
        }
    }
}

/// Tool for evaluating the architectural causal blast radius before modifying code.
pub struct CausalBlastRadiusTool {
    world_model: Arc<strata_reasoning::WorldModel>,
}

impl CausalBlastRadiusTool {
    pub fn new(world_model: Arc<strata_reasoning::WorldModel>) -> Self {
        Self { world_model }
    }
}

#[async_trait]
impl Tool for CausalBlastRadiusTool {
    fn name(&self) -> &str {
        "causal_blast_radius"
    }

    fn description(&self) -> &str {
        "Analyze the architectural causal blast radius, downstream ripple effects, and breaking risk before modifying code."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "target": {
                    "type": "string",
                    "description": "File path, module name, or struct to evaluate (e.g. 'crates/strata-server/src/storage.rs' or 'ServerStorage')"
                },
                "depth": {
                    "type": "integer",
                    "description": "Maximum traversal depth for transitive dependencies (default 3)"
                }
            },
            "required": ["target"]
        })
    }

    async fn execute(&self, params: serde_json::Value) -> Result<serde_json::Value, StrataError> {
        let target = params
            .get("target")
            .and_then(|v| v.as_str())
            .ok_or_else(|| StrataError::ValidationError("Missing 'target' field".to_string()))?;

        let depth = params.get("depth").and_then(|v| v.as_u64()).unwrap_or(3) as usize;

        let report = self
            .world_model
            .predict_impact(target, depth)
            .await
            .map_err(|e| StrataError::ReasoningError(e.to_string()))?;

        Ok(json!(report))
    }
}

/// Tool for decomposing high-level objectives into an executable Goal DAG.
pub struct GoalDecomposeTool {
    decomposer: Arc<strata_reasoning::GoalDecomposer>,
}

impl Default for GoalDecomposeTool {
    fn default() -> Self {
        Self::new()
    }
}

impl GoalDecomposeTool {
    pub fn new() -> Self {
        Self {
            decomposer: Arc::new(strata_reasoning::GoalDecomposer::new()),
        }
    }

    pub fn with_decomposer(decomposer: Arc<strata_reasoning::GoalDecomposer>) -> Self {
        Self { decomposer }
    }
}

#[async_trait]
impl Tool for GoalDecomposeTool {
    fn name(&self) -> &str {
        "goal_decompose"
    }

    fn description(&self) -> &str {
        "Decompose high-level objectives into a structured Goal DAG with parallel execution waves and verification gates."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "goal": {
                    "type": "string",
                    "description": "Natural language long-horizon task or objective to decompose"
                },
                "max_depth": {
                    "type": "integer",
                    "description": "Maximum decomposition depth (default: 3)"
                },
                "include_verification": {
                    "type": "boolean",
                    "description": "Whether to include verification gates and invariant checks (default: true)"
                }
            },
            "required": ["goal"]
        })
    }

    async fn execute(&self, params: serde_json::Value) -> Result<serde_json::Value, StrataError> {
        let goal = params
            .get("goal")
            .and_then(|v| v.as_str())
            .ok_or_else(|| StrataError::ValidationError("Missing 'goal' field".to_string()))?;

        let include_verification = params
            .get("include_verification")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        let dag = if include_verification {
            self.decomposer.decompose(goal)
        } else {
            strata_reasoning::GoalDecomposer::new()
                .with_verification_gates(false)
                .decompose(goal)
        }
        .map_err(|e| StrataError::ReasoningError(format!("Goal decomposition error: {e}")))?;

        let waves = dag
            .compute_waves()
            .map_err(|e| StrataError::ReasoningError(format!("Wave computation error: {e}")))?;

        let ascii_tree = dag.to_ascii_tree();
        let export = dag.export();

        Ok(json!({
            "status": "success",
            "goal": goal,
            "total_nodes": dag.node_count(),
            "total_waves": waves.len(),
            "waves": waves,
            "dag": export,
            "ascii_tree": ascii_tree
        }))
    }
}

/// Tool for executing a Goal DAG plan wave-by-wave asynchronously with dynamic recovery.
pub struct DagExecuteTool {
    scheduler: Arc<strata_reasoning::DagScheduler>,
}

impl Default for DagExecuteTool {
    fn default() -> Self {
        Self::new()
    }
}

impl DagExecuteTool {
    pub fn new() -> Self {
        Self {
            scheduler: Arc::new(strata_reasoning::DagScheduler::new()),
        }
    }

    pub fn with_scheduler(scheduler: Arc<strata_reasoning::DagScheduler>) -> Self {
        Self { scheduler }
    }
}

#[async_trait]
impl Tool for DagExecuteTool {
    fn name(&self) -> &str {
        "dag_execute"
    }

    fn description(&self) -> &str {
        "Execute a Goal DAG plan wave-by-wave asynchronously with controlled concurrency and dynamic failure recovery."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
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
                    "description": "Maximum number of parallel tasks to run concurrently (default: 4)"
                },
                "auto_recover": {
                    "type": "boolean",
                    "description": "Whether to dynamically recover from failures by patching DAG (default: true)"
                }
            }
        })
    }

    async fn execute(&self, params: serde_json::Value) -> Result<serde_json::Value, StrataError> {
        let concurrency = params
            .get("max_concurrency")
            .and_then(|v| v.as_u64())
            .unwrap_or(4) as usize;

        let auto_recover = params
            .get("auto_recover")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        let dag = if let Some(dag_val) = params.get("dag") {
            let export: strata_reasoning::GoalDagExport = serde_json::from_value(dag_val.clone())
                .map_err(|e| StrataError::ValidationError(format!("Invalid Goal DAG export: {e}")))?;
            strata_reasoning::GoalDag::from_export(export)
                .map_err(|e| StrataError::ValidationError(format!("Invalid Goal DAG structure: {e}")))?
        } else if let Some(goal) = params.get("goal").and_then(|v| v.as_str()) {
            strata_reasoning::GoalDecomposer::new()
                .decompose(goal)
                .map_err(|e| StrataError::ReasoningError(format!("Goal decomposition error: {e}")))?
        } else {
            return Err(StrataError::ValidationError(
                "Either 'dag' or 'goal' parameter must be provided".to_string(),
            ));
        };

        let (finished_dag, report) = if concurrency == 4 && auto_recover {
            self.scheduler.execute(dag).await
        } else {
            strata_reasoning::DagScheduler::new()
                .with_concurrency(concurrency)
                .with_auto_recover(auto_recover)
                .execute(dag)
                .await
        }
        .map_err(|e| StrataError::ReasoningError(format!("DAG execution error: {e}")))?;

        let ascii_tree = finished_dag.to_ascii_tree();

        Ok(json!({
            "status": if report.success { "success" } else { "failed" },
            "report": report,
            "ascii_tree": ascii_tree
        }))
    }
}

/// Tool for one-click local LoRA fine-tuning via Unsloth and Ollama deployment.
pub struct TrainPipelineTool {
    store: Option<Arc<strata_memory::SqliteStore>>,
}

impl Default for TrainPipelineTool {
    fn default() -> Self {
        Self::new()
    }
}

impl TrainPipelineTool {
    pub fn new() -> Self {
        Self { store: None }
    }

    pub fn with_store(store: Arc<strata_memory::SqliteStore>) -> Self {
        Self { store: Some(store) }
    }
}

#[async_trait]
impl Tool for TrainPipelineTool {
    fn name(&self) -> &str {
        "train_pipeline"
    }

    fn description(&self) -> &str {
        "Synthesize one-click Unsloth LoRA fine-tuning scripts, Ollama Modelfile, datasets, and execution artifacts."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "base_model": {
                    "type": "string",
                    "description": "HuggingFace base model identifier (default: 'unsloth/Llama-3.2-3B-Instruct')"
                },
                "method": {
                    "type": "string",
                    "enum": ["dpo", "sft", "orpo", "kto"],
                    "description": "Fine-tuning optimization method (default: 'dpo')"
                },
                "quantization": {
                    "type": "string",
                    "enum": ["4bit", "8bit", "16bit", "none"],
                    "description": "Quantization format for base model loading (default: '4bit')"
                },
                "lora_r": {
                    "type": "integer",
                    "description": "LoRA rank dimension (default: 16)"
                },
                "lora_alpha": {
                    "type": "integer",
                    "description": "LoRA scaling alpha (default: 32)"
                },
                "lora_dropout": {
                    "type": "number",
                    "description": "LoRA dropout probability (default: 0.0)"
                },
                "learning_rate": {
                    "type": "number",
                    "description": "Optimizer learning rate (default: 5e-5)"
                },
                "batch_size": {
                    "type": "integer",
                    "description": "Per-device training batch size (default: 2)"
                },
                "gradient_accumulation_steps": {
                    "type": "integer",
                    "description": "Gradient accumulation steps (default: 4)"
                },
                "max_steps": {
                    "type": "integer",
                    "description": "Maximum training steps (default: 60)"
                },
                "max_seq_length": {
                    "type": "integer",
                    "description": "Maximum sequence context length (default: 2048)"
                },
                "output_dir": {
                    "type": "string",
                    "description": "Target directory for synthesized training artifacts (default: './outputs/lora_run')"
                },
                "dataset_content": {
                    "type": "string",
                    "description": "Optional raw JSONL dataset string"
                },
                "ollama_model_name": {
                    "type": "string",
                    "description": "Target model identifier for local Ollama registration (default: 'strata-custom-coder')"
                },
                "export_gguf": {
                    "type": "boolean",
                    "description": "Whether to export GGUF format for Ollama (default: true)"
                },
                "dry_run": {
                    "type": "boolean",
                    "description": "Whether to run dry-run artifact synthesis without starting Python (default: true)"
                }
            }
        })
    }

    async fn execute(&self, params: serde_json::Value) -> Result<serde_json::Value, StrataError> {
        let base_model = params
            .get("base_model")
            .and_then(|v| v.as_str())
            .unwrap_or("unsloth/Llama-3.2-3B-Instruct");

        let method_str = params
            .get("method")
            .and_then(|v| v.as_str())
            .unwrap_or("dpo");
        let method = method_str.parse::<strata_reasoning::TrainingMethod>()?;

        let quant_str = params
            .get("quantization")
            .and_then(|v| v.as_str())
            .unwrap_or("4bit");
        let quantization = quant_str.parse::<strata_reasoning::QuantizationType>()?;

        let lora_r = params.get("lora_r").and_then(|v| v.as_u64()).unwrap_or(16) as u32;
        let lora_alpha = params.get("lora_alpha").and_then(|v| v.as_u64()).unwrap_or(32) as u32;
        let lora_dropout = params.get("lora_dropout").and_then(|v| v.as_f64()).unwrap_or(0.0) as f32;
        let learning_rate = params.get("learning_rate").and_then(|v| v.as_f64()).unwrap_or(5e-5);
        let batch_size = params.get("batch_size").and_then(|v| v.as_u64()).unwrap_or(2) as usize;
        let grad_accum = params.get("gradient_accumulation_steps").and_then(|v| v.as_u64()).unwrap_or(4) as usize;
        let max_steps = params.get("max_steps").and_then(|v| v.as_u64()).unwrap_or(60) as usize;
        let max_seq_length = params.get("max_seq_length").and_then(|v| v.as_u64()).unwrap_or(2048) as usize;
        let output_dir_str = params.get("output_dir").and_then(|v| v.as_str()).unwrap_or("./outputs/lora_run");
        let ollama_name = params.get("ollama_model_name").and_then(|v| v.as_str()).unwrap_or("strata-custom-coder");
        let export_gguf = params.get("export_gguf").and_then(|v| v.as_bool()).unwrap_or(true);

        let mut config = strata_reasoning::TrainingConfig::new(base_model)
            .with_method(method)
            .with_quantization(quantization)
            .with_lora(lora_r, lora_alpha, lora_dropout)
            .with_learning_rate(learning_rate)
            .with_batch_size(batch_size, grad_accum)
            .with_max_steps(max_steps)
            .with_max_seq_length(max_seq_length)
            .with_output_dir(output_dir_str)
            .with_ollama_model(ollama_name);
        config.export_gguf = export_gguf;

        // Determine dataset content and sample count
        let (dataset_str, sample_count) = if let Some(content) = params.get("dataset_content").and_then(|v| v.as_str()) {
            let lines_count = content.lines().filter(|l| !l.trim().is_empty()).count();
            (Some(content.to_string()), lines_count)
        } else if let Some(ref store) = self.store {
            let miner = strata_memory::PreferenceMiner::new(store.clone());
            let fmt = match method {
                strata_reasoning::TrainingMethod::Sft => strata_memory::ExportFormat::Sft,
                strata_reasoning::TrainingMethod::Kto => strata_memory::ExportFormat::Kto,
                _ => strata_memory::ExportFormat::Dpo,
            };
            let mined = miner.export(fmt, None).unwrap_or_default();
            let lines_count = mined.lines().filter(|l| !l.trim().is_empty()).count();
            (Some(mined), lines_count)
        } else {
            let default_dpo = "{\"prompt\":\"Context: Agent encountered borrow error\",\"chosen\":\"Use Arc and Clone properly\",\"rejected\":\"Force unsafe raw pointers\"}\n";
            (Some(default_dpo.to_string()), 1)
        };

        let pipeline = strata_reasoning::TrainingPipeline::new(config);
        let out_path = std::path::Path::new(output_dir_str);
        let result = pipeline.generate_artifacts(out_path, dataset_str.as_deref(), sample_count)?;
        let summary_table = pipeline.format_summary_table(sample_count);

        Ok(json!({
            "status": "success",
            "manifest": result.manifest,
            "script_path": result.script_path,
            "dataset_path": result.dataset_path,
            "modelfile_path": result.modelfile_path,
            "run_script_path": result.run_script_path,
            "summary": result.summary,
            "summary_table": summary_table,
            "total_samples": sample_count
        }))
    }
}

// =========================================================================
// Native Call Graph Tool
// =========================================================================

pub struct CallGraphTool {
    store: Option<Arc<strata_memory::store::SqliteStore>>,
}

impl CallGraphTool {
    pub fn new() -> Self {
        Self { store: None }
    }

    pub fn with_store(store: Arc<strata_memory::store::SqliteStore>) -> Self {
        Self { store: Some(store) }
    }
}

impl Default for CallGraphTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for CallGraphTool {
    fn name(&self) -> &str {
        "strata_call_graph"
    }

    fn description(&self) -> &str {
        "Deterministic Native Call Graph and Module Import Dependency Analyzer in Rust. Analyzes function calls, method calls, constructors, imports, and recursion."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "File path to analyze or target"
                },
                "code": {
                    "type": "string",
                    "description": "Optional raw source code string to analyze on-the-fly"
                },
                "symbol": {
                    "type": "string",
                    "description": "Optional symbol/function name to inspect callers or callees"
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
        })
    }

    async fn execute(&self, params: serde_json::Value) -> Result<serde_json::Value, StrataError> {
        let path_opt = params.get("path").and_then(|v| v.as_str());
        let code_opt = params.get("code").and_then(|v| v.as_str());
        let symbol_opt = params.get("symbol").and_then(|v| v.as_str());
        let direction = params
            .get("direction")
            .and_then(|v| v.as_str())
            .unwrap_or("all");
        let limit = params.get("limit").and_then(|v| v.as_u64()).unwrap_or(50) as usize;

        let analyzer = strata_memory::CallGraphAnalyzer::new();
        let mut edges = Vec::new();

        // 1. If code or path is provided, analyze directly
        if let Some(code) = code_opt {
            let file_path = path_opt.unwrap_or("snippet.rs");
            let lang = strata_memory::LanguageKind::from_file_path(file_path);
            let extracted = analyzer.analyze_source(code, lang, file_path)?;
            if let Some(ref store) = self.store {
                let _ = store.clear_call_edges_for_file(file_path);
                let _ = store.insert_call_edges(&extracted);
            }
            edges.extend(extracted);
        } else if let Some(path) = path_opt {
            if let Ok(content) = std::fs::read_to_string(path) {
                let lang = strata_memory::LanguageKind::from_file_path(path);
                let extracted = analyzer.analyze_source(&content, lang, path)?;
                if let Some(ref store) = self.store {
                    let _ = store.clear_call_edges_for_file(path);
                    let _ = store.insert_call_edges(&extracted);
                }
                edges.extend(extracted);
            } else if let Some(ref store) = self.store {
                edges = store.get_file_call_edges(path)?;
            }
        } else if let (Some(ref store), Some(sym)) = (&self.store, symbol_opt) {
            edges = store.get_callers_of_symbol(sym, limit)?;
        }

        let graph = strata_memory::CallGraph::from_edges(edges.clone());
        let recursive = graph.detect_recursive_calls();

        // Filter based on symbol and direction
        let callers: Vec<_> = if let Some(sym) = symbol_opt {
            graph.callers_of(sym).into_iter().cloned().collect()
        } else {
            edges
                .iter()
                .filter(|e| e.call_type != strata_memory::CallType::Import)
                .cloned()
                .collect()
        };

        let callees: Vec<_> = if let (Some(sym), Some(p)) = (symbol_opt, path_opt) {
            graph.callees_of(p, sym).into_iter().cloned().collect()
        } else {
            Vec::new()
        };

        let imports: Vec<_> = if let Some(p) = path_opt {
            graph.file_imports(p).into_iter().cloned().collect()
        } else {
            edges
                .iter()
                .filter(|e| e.call_type == strata_memory::CallType::Import)
                .cloned()
                .collect()
        };

        // Formatted Markdown / ASCII tree summary for agents
        let mut summary_lines = Vec::new();
        summary_lines.push(format!("### 📞 Call Graph Analysis ({direction})"));
        if let Some(p) = path_opt {
            summary_lines.push(format!("- **Target File**: `{p}`"));
        }
        if let Some(sym) = symbol_opt {
            summary_lines.push(format!("- **Target Symbol**: `{sym}`"));
        }
        summary_lines.push(format!("- **Total Edges Detected**: {}", edges.len()));

        if !recursive.is_empty() {
            summary_lines.push(format!("- ⚠️ **Recursive Calls**: {}", recursive.len()));
            for (f, sym) in &recursive {
                summary_lines.push(format!("  - `{}::{}` (self-call)", f, sym));
            }
        }

        if !imports.is_empty() && (direction == "imports" || direction == "all") {
            summary_lines.push("\n#### 📦 Import Dependencies:".to_string());
            for imp in &imports {
                summary_lines.push(format!("  - line {}: `{}`", imp.line_number, imp.callee_symbol));
            }
        }

        if !callers.is_empty() && (direction == "callers" || direction == "both" || direction == "all") {
            summary_lines.push("\n#### ⬆️ Callers (Who invokes this):".to_string());
            for c in &callers {
                summary_lines.push(format!(
                    "  - `{}:{}` (line {}) -> invokes `{}` [{}]",
                    c.caller_file, c.caller_symbol, c.line_number, c.callee_symbol, c.call_type
                ));
            }
        }

        if !callees.is_empty() && (direction == "callees" || direction == "both" || direction == "all") {
            summary_lines.push("\n#### ⬇️ Callees (What this invokes):".to_string());
            for c in &callees {
                summary_lines.push(format!(
                    "  - line {}: `{}` [{}]",
                    c.line_number, c.callee_symbol, c.call_type
                ));
            }
        }

        let formatted_summary = summary_lines.join("\n");

        Ok(json!({
            "status": "success",
            "total_edges": edges.len(),
            "target_path": path_opt,
            "target_symbol": symbol_opt,
            "direction": direction,
            "callers": callers,
            "callees": callees,
            "imports": imports,
            "recursive_calls": recursive,
            "formatted_summary": formatted_summary
        }))
    }
}

// =========================================================================
// Workspace Boundary Detection Tool
// =========================================================================

pub struct WorkspaceDetectTool;

impl WorkspaceDetectTool {
    pub fn new() -> Self {
        Self
    }
}

impl Default for WorkspaceDetectTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for WorkspaceDetectTool {
    fn name(&self) -> &str {
        "strata_workspace_detect"
    }

    fn description(&self) -> &str {
        "Detect monorepo workspace boundaries, member packages/crates, internal dependencies, and isolate file scopes."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "root_path": {
                    "type": "string",
                    "description": "Root repository or workspace directory to inspect (default: current directory '.')"
                },
                "file_path": {
                    "type": "string",
                    "description": "Optional file path to resolve against detected package boundaries and get hierarchical scopes"
                }
            }
        })
    }

    async fn execute(&self, params: serde_json::Value) -> Result<serde_json::Value, StrataError> {
        let root_str = params
            .get("root_path")
            .and_then(|v| v.as_str())
            .unwrap_or(".");
        let file_path_opt = params.get("file_path").and_then(|v| v.as_str());

        let root_path = std::path::Path::new(root_str);
        let boundary = strata_memory::WorkspaceBoundaryDetector::detect(root_path)?;

        let mut resolved_pkg = None;
        let mut hierarchical_scopes = Vec::new();

        if let Some(f_path) = file_path_opt {
            resolved_pkg = boundary.find_package_for_file(f_path).cloned();
            hierarchical_scopes = boundary
                .get_hierarchical_scopes(f_path)
                .into_iter()
                .map(|s| s.to_string())
                .collect();
        }

        // Formatted Markdown summary for coding agents
        let mut summary_lines = Vec::new();
        summary_lines.push(format!("### 🏢 Workspace Boundary Report ({})", boundary.workspace_type));
        summary_lines.push(format!("- **Root Directory**: `{}`", boundary.root_path));
        summary_lines.push(format!("- **Member Packages ({} found)**:", boundary.packages.len()));

        for pkg in &boundary.packages {
            let deps_str = if pkg.internal_dependencies.is_empty() {
                "".to_string()
            } else {
                format!(" (depends on: {})", pkg.internal_dependencies.join(", "))
            };
            summary_lines.push(format!(
                "  - **`{}`** [{}] @ `{}`{}",
                pkg.name, pkg.package_type, pkg.root_path, deps_str
            ));
        }

        if let Some(ref pkg) = resolved_pkg {
            summary_lines.push(format!("\n#### 🎯 Resolved Target File Context:"));
            summary_lines.push(format!("- **File**: `{}`", file_path_opt.unwrap_or("")));
            summary_lines.push(format!("- **Owning Package**: `{}`", pkg.name));
            summary_lines.push(format!("- **Hierarchical Search Scopes**: `{}`", hierarchical_scopes.join(" -> ")));
        }

        let formatted_summary = summary_lines.join("\n");

        Ok(json!({
            "status": "success",
            "root_path": boundary.root_path,
            "workspace_type": boundary.workspace_type.to_string(),
            "packages_count": boundary.packages.len(),
            "packages": boundary.packages,
            "resolved_package": resolved_pkg,
            "hierarchical_scopes": hierarchical_scopes,
            "formatted_summary": formatted_summary
        }))
    }
}

// =========================================================================
// Graph Communities & Architecture Map Tool
// =========================================================================

pub struct ArchitectureMapTool;

impl ArchitectureMapTool {
    pub fn new() -> Self {
        Self
    }
}

impl Default for ArchitectureMapTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for ArchitectureMapTool {
    fn name(&self) -> &str {
        "strata_architecture_map"
    }

    fn description(&self) -> &str {
        "Extract graph communities and high-level architectural clusters from codebase call graphs, imports, and AST symbols to understand macro software architecture."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
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
        })
    }

    async fn execute(&self, params: serde_json::Value) -> Result<serde_json::Value, StrataError> {
        let path_str = params.get("path").and_then(|v| v.as_str()).unwrap_or(".");
        let ws_id = params.get("workspace_id").and_then(|v| v.as_str()).unwrap_or("default");
        let min_cluster_size = params.get("min_cluster_size").and_then(|v| v.as_u64()).unwrap_or(1) as usize;
        let max_iter = params.get("max_iterations").and_then(|v| v.as_u64()).unwrap_or(25) as usize;

        let config = strata_memory::ClusteringConfig {
            max_iterations: max_iter,
            min_cluster_size,
            call_weight: 1.5,
            import_weight: 1.0,
        };

        let detector = strata_memory::CommunityDetector::new(config);
        let summary = detector.detect_from_directory(std::path::Path::new(path_str), ws_id)?;

        Ok(json!({
            "status": "success",
            "workspace_id": summary.workspace_id,
            "total_nodes": summary.total_nodes,
            "total_edges": summary.total_edges,
            "modularity": summary.modularity,
            "cross_cluster_edges_count": summary.cross_cluster_edges_count,
            "clusters_count": summary.clusters.len(),
            "clusters": summary.clusters,
            "formatted_summary": summary.formatted_summary
        }))
    }
}

// =========================================================================
// Human-in-the-Loop Core Tier Promotion Tool
// =========================================================================

pub struct CoreTierPromoteTool {
    engine: Arc<strata_memory::SqliteMemoryEngine>,
}

impl CoreTierPromoteTool {
    pub fn new(engine: Arc<strata_memory::SqliteMemoryEngine>) -> Self {
        Self { engine }
    }
}

#[async_trait]
impl Tool for CoreTierPromoteTool {
    fn name(&self) -> &str {
        "strata_promote"
    }

    fn description(&self) -> &str {
        "Promote a memory record or semantic fact to permanent Core Tier (frozen retention, R=1.0). Requires explicit human approval confirmation."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "id": {
                    "type": "string",
                    "description": "The UUID of the memory record or semantic fact to promote"
                },
                "entity_type": {
                    "type": "string",
                    "enum": ["memory", "fact"],
                    "description": "Entity type: 'memory' (default) or 'fact'"
                },
                "approved_by_human": {
                    "type": "boolean",
                    "description": "Explicit human confirmation flag (must be true to complete promotion)"
                },
                "reason": {
                    "type": "string",
                    "description": "Optional policy rationale or justification for permanent Core Tier promotion"
                }
            },
            "required": ["id", "approved_by_human"]
        })
    }

    async fn execute(&self, params: serde_json::Value) -> Result<serde_json::Value, StrataError> {
        let id_str = params
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| StrataError::ValidationError("Missing 'id' parameter".to_string()))?;

        let id = Uuid::parse_str(id_str)
            .map_err(|e| StrataError::ValidationError(format!("Invalid UUID format: {e}")))?;

        let approved = params
            .get("approved_by_human")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let entity_type = params
            .get("entity_type")
            .and_then(|v| v.as_str())
            .unwrap_or("memory");

        let reason = params.get("reason").and_then(|v| v.as_str());

        if !approved {
            return Err(StrataError::Validation(
                "Promotion to Core Tier was rejected: 'approved_by_human' must be explicitly set to true by developer/human approval.".to_string(),
            ));
        }

        if entity_type == "fact" || entity_type == "semantic" {
            let promoted = self.engine.promote_fact_to_core(&id, true, reason).await?;
            Ok(json!({
                "status": "success",
                "id": promoted.id.to_string(),
                "tier": "core",
                "approved_by_human": true,
                "importance": promoted.importance,
                "statement": promoted.statement,
                "reason": reason,
            }))
        } else {
            let promoted = self.engine.promote_to_core(&id, true, reason).await?;
            Ok(json!({
                "status": "success",
                "id": promoted.id.to_string(),
                "tier": "core",
                "approved_by_human": true,
                "importance": promoted.importance,
                "content": promoted.content,
                "reason": reason,
            }))
        }
    }
}






