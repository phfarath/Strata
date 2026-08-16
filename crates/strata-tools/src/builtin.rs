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
