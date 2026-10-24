use chrono::Utc;
use serde::{Deserialize, Serialize};

/// Lifecycle status of a task or subtask in the shared blackboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    /// Task is awaiting prerequisite dependencies or waiting to be claimed
    Pending,
    /// Task is claimed by an agent with active resource leases
    Claimed,
    /// Task is actively being executed
    Running,
    /// Task is waiting for prerequisite tasks to complete
    Blocked,
    /// Task successfully finished, outputs persisted, downstream tasks unblocked
    Completed,
    /// Task failed execution or verification
    Failed,
}

impl TaskStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Claimed => "claimed",
            Self::Running => "running",
            Self::Blocked => "blocked",
            Self::Completed => "completed",
            Self::Failed => "failed",
        }
    }
}

impl std::fmt::Display for TaskStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl std::str::FromStr for TaskStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "pending" => Ok(Self::Pending),
            "claimed" => Ok(Self::Claimed),
            "running" => Ok(Self::Running),
            "blocked" => Ok(Self::Blocked),
            "completed" => Ok(Self::Completed),
            "failed" => Ok(Self::Failed),
            other => Err(format!("Unknown task status: {other}")),
        }
    }
}

/// The explicit type or executable payload of a task on the blackboard.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
pub enum TaskPayload {
    /// A deterministic shell command to execute in a subprocess
    ShellCommand {
        command: String,
        #[serde(default)]
        cwd: Option<String>,
        #[serde(default)]
        timeout_secs: Option<u64>,
    },
    /// An autonomous agent prompt requiring LLM reasoning/coding
    AgentPrompt {
        prompt: String,
        #[serde(default)]
        context_files: Vec<String>,
        #[serde(default)]
        expected_artifacts: Vec<String>,
    },
    /// A manual verification step requiring human sign-off
    ManualVerification {
        instructions: String,
        #[serde(default)]
        checklist: Vec<String>,
    },
    /// A descriptive / generic task without an automated executable command
    Descriptive {
        #[serde(default)]
        details: Option<String>,
    },
}

impl TaskPayload {
    pub fn payload_type(&self) -> &'static str {
        match self {
            Self::ShellCommand { .. } => "shell_command",
            Self::AgentPrompt { .. } => "agent_prompt",
            Self::ManualVerification { .. } => "manual_verification",
            Self::Descriptive { .. } => "descriptive",
        }
    }

    pub fn is_executable_by_worker(&self) -> bool {
        matches!(self, Self::ShellCommand { .. })
    }
}

/// A persistent, shared task record on the multi-agent blackboard.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TaskRecord {
    pub task_id: String,
    pub plan_id: Option<String>,
    pub parent_id: Option<String>,
    pub title: String,
    pub description: Option<String>,
    pub status: TaskStatus,
    pub assigned_agent: Option<String>,
    pub required_resources: Vec<String>,
    pub dependencies: Vec<String>,
    pub outputs: Option<serde_json::Value>,
    pub created_at: i64,
    pub updated_at: i64,
    #[serde(default)]
    pub claim_generation: i64,
    #[serde(default)]
    pub payload: Option<TaskPayload>,
}

impl TaskRecord {
    pub fn new(task_id: impl Into<String>, title: impl Into<String>) -> Self {
        let now = Utc::now().timestamp();
        Self {
            task_id: task_id.into(),
            plan_id: None,
            parent_id: None,
            title: title.into(),
            description: None,
            status: TaskStatus::Pending,
            assigned_agent: None,
            required_resources: Vec::new(),
            dependencies: Vec::new(),
            outputs: None,
            created_at: now,
            updated_at: now,
            claim_generation: 0,
            payload: None,
        }
    }

    pub fn with_payload(mut self, payload: TaskPayload) -> Self {
        self.payload = Some(payload);
        self
    }

    pub fn with_plan_id(mut self, plan_id: impl Into<String>) -> Self {
        self.plan_id = Some(plan_id.into());
        self
    }

    pub fn with_parent_id(mut self, parent_id: impl Into<String>) -> Self {
        self.parent_id = Some(parent_id.into());
        self
    }

    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    pub fn with_status(mut self, status: TaskStatus) -> Self {
        self.status = status;
        self
    }

    pub fn with_required_resources(mut self, resources: Vec<String>) -> Self {
        self.required_resources = resources;
        self
    }

    pub fn with_dependencies(mut self, dependencies: Vec<String>) -> Self {
        self.dependencies = dependencies;
        self
    }

    pub fn is_ready(&self, completed_tasks: &[String]) -> bool {
        self.dependencies
            .iter()
            .all(|dep| completed_tasks.contains(dep))
    }

    /// Resolves the effective task payload, prioritizing explicit `payload`,
    /// then checking structured outputs, and finally falling back to heuristic
    /// parsing of the description (`cmd:`, `run:`, `$ `, or JSON).
    pub fn resolve_payload(&self) -> TaskPayload {
        if let Some(ref p) = self.payload {
            return p.clone();
        }

        if let Some(ref outputs) = self.outputs {
            if let Ok(p) = serde_json::from_value::<TaskPayload>(outputs.clone()) {
                return p;
            }
        }

        if let Some(ref desc) = self.description {
            let trimmed_desc = desc.trim();
            if trimmed_desc.starts_with('{') && trimmed_desc.ends_with('}') {
                if let Ok(p) = serde_json::from_str::<TaskPayload>(trimmed_desc) {
                    return p;
                }
            }

            for line in desc.lines() {
                let trimmed = line.trim();
                if let Some(cmd) = trimmed.strip_prefix("cmd:") {
                    return TaskPayload::ShellCommand {
                        command: cmd.trim().to_string(),
                        cwd: None,
                        timeout_secs: None,
                    };
                }
                if let Some(cmd) = trimmed.strip_prefix("run:") {
                    return TaskPayload::ShellCommand {
                        command: cmd.trim().to_string(),
                        cwd: None,
                        timeout_secs: None,
                    };
                }
                if let Some(cmd) = trimmed.strip_prefix("$ ") {
                    return TaskPayload::ShellCommand {
                        command: cmd.trim().to_string(),
                        cwd: None,
                        timeout_secs: None,
                    };
                }
            }

            TaskPayload::Descriptive {
                details: Some(desc.clone()),
            }
        } else {
            TaskPayload::Descriptive { details: None }
        }
    }

    pub fn payload_type(&self) -> &'static str {
        self.resolve_payload().payload_type()
    }

    pub fn is_executable_by_worker(&self) -> bool {
        self.resolve_payload().is_executable_by_worker()
    }
}

/// A durable artifact generated during task execution for cross-agent handoff.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TaskArtifact {
    pub artifact_id: String,
    pub task_id: String,
    pub agent_id: String,
    pub kind: String, // "summary", "diff", "code", "json"
    pub payload: String,
    pub created_at: i64,
}

impl TaskArtifact {
    pub fn new(
        artifact_id: impl Into<String>,
        task_id: impl Into<String>,
        agent_id: impl Into<String>,
        kind: impl Into<String>,
        payload: impl Into<String>,
    ) -> Self {
        Self {
            artifact_id: artifact_id.into(),
            task_id: task_id.into(),
            agent_id: agent_id.into(),
            kind: kind.into(),
            payload: payload.into(),
            created_at: Utc::now().timestamp(),
        }
    }
}

/// Criteria for filtering tasks in queries.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskFilter {
    pub plan_id: Option<String>,
    pub status: Option<TaskStatus>,
    pub assigned_agent: Option<String>,
    pub limit: Option<usize>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_task_status_string_conversion() {
        assert_eq!(TaskStatus::Pending.as_str(), "pending");
        assert_eq!(
            "pending".parse::<TaskStatus>().unwrap(),
            TaskStatus::Pending
        );
        assert_eq!(
            "completed".parse::<TaskStatus>().unwrap(),
            TaskStatus::Completed
        );
        assert!("invalid".parse::<TaskStatus>().is_err());
    }

    #[test]
    fn test_task_record_lifecycle() {
        let task = TaskRecord::new("subtask-1", "Implement database schema")
            .with_plan_id("plan-100")
            .with_required_resources(vec!["crate:strata-memory".to_string()])
            .with_dependencies(vec!["subtask-0".to_string()]);

        assert_eq!(task.task_id, "subtask-1");
        assert_eq!(task.status, TaskStatus::Pending);
        assert!(!task.is_ready(&[]));
        assert!(task.is_ready(&["subtask-0".to_string()]));

        let json = serde_json::to_string(&task).expect("serialize task");
        let de: TaskRecord = serde_json::from_str(&json).expect("deserialize task");
        assert_eq!(task, de);
    }

    #[test]
    fn test_task_artifact_serialization() {
        let artifact = TaskArtifact::new(
            "art-1",
            "subtask-1",
            "agent-claude",
            "summary",
            "Implemented tables: tasks, a2a_messages",
        );

        let json = serde_json::to_string(&artifact).expect("serialize artifact");
        let de: TaskArtifact = serde_json::from_str(&json).expect("deserialize artifact");
        assert_eq!(artifact, de);
    }

    #[test]
    fn test_task_payload_types_and_resolution() {
        // 1. Explicit ShellCommand payload
        let cmd_task =
            TaskRecord::new("t-1", "Run unit tests").with_payload(TaskPayload::ShellCommand {
                command: "cargo test --workspace".to_string(),
                cwd: Some("/workspace".to_string()),
                timeout_secs: Some(120),
            });
        assert_eq!(cmd_task.payload_type(), "shell_command");
        assert!(cmd_task.is_executable_by_worker());

        // 2. Explicit AgentPrompt payload
        let prompt_task =
            TaskRecord::new("t-2", "Design auth flow").with_payload(TaskPayload::AgentPrompt {
                prompt: "Implement JWT refresh flow".to_string(),
                context_files: vec!["src/auth.rs".to_string()],
                expected_artifacts: vec!["src/token.rs".to_string()],
            });
        assert_eq!(prompt_task.payload_type(), "agent_prompt");
        assert!(!prompt_task.is_executable_by_worker());

        // 3. Explicit ManualVerification payload
        let verify_task =
            TaskRecord::new("t-3", "QA Signoff").with_payload(TaskPayload::ManualVerification {
                instructions: "Verify login screen on mobile device".to_string(),
                checklist: vec!["Check error banner".to_string()],
            });
        assert_eq!(verify_task.payload_type(), "manual_verification");
        assert!(!verify_task.is_executable_by_worker());

        // 4. Backward-compatible heuristic: 'cmd:' prefix in description
        let desc_cmd_task = TaskRecord::new("t-4", "Build crate")
            .with_description("Building core crate\ncmd: cargo build -p strata-core");
        assert_eq!(desc_cmd_task.payload_type(), "shell_command");
        assert!(desc_cmd_task.is_executable_by_worker());
        if let TaskPayload::ShellCommand { command, .. } = desc_cmd_task.resolve_payload() {
            assert_eq!(command, "cargo build -p strata-core");
        } else {
            panic!("Expected ShellCommand");
        }

        // 5. Backward-compatible heuristic: '$ ' prefix in description
        let dollar_cmd_task =
            TaskRecord::new("t-5", "Check linter").with_description("$ cargo clippy --all-targets");
        assert_eq!(dollar_cmd_task.payload_type(), "shell_command");
        assert!(dollar_cmd_task.is_executable_by_worker());

        // 6. Purely descriptive task without command: must resolve to Descriptive
        let descriptive_task = TaskRecord::new("t-6", "Refactor state machine")
            .with_description("Refactor the state machine to use typed transitions and states");
        assert_eq!(descriptive_task.payload_type(), "descriptive");
        assert!(!descriptive_task.is_executable_by_worker());

        // 7. Serialization round-trip
        let json = serde_json::to_string(&cmd_task).expect("serialize cmd task");
        let de: TaskRecord = serde_json::from_str(&json).expect("deserialize cmd task");
        assert_eq!(cmd_task, de);
    }
}
