use std::collections::HashMap;
use serde::{Deserialize, Serialize};

/// Categorization of goal nodes in the hierarchical planning DAG.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum GoalNodeKind {
    /// Top-level root objective.
    Root,
    /// Broad phase or major milestone containing subtasks.
    Phase,
    /// Concrete executable unit of work.
    Task,
    /// Verification gate, quality check, or invariant test.
    Verification,
    /// Compensating or recovery rollback action.
    Rollback,
}

impl std::fmt::Display for GoalNodeKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GoalNodeKind::Root => write!(f, "root"),
            GoalNodeKind::Phase => write!(f, "phase"),
            GoalNodeKind::Task => write!(f, "task"),
            GoalNodeKind::Verification => write!(f, "verification"),
            GoalNodeKind::Rollback => write!(f, "rollback"),
        }
    }
}

/// Lifecycle status of a goal node.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum GoalStatus {
    /// Ready for execution once prerequisites are met.
    Pending,
    /// Currently actively executing.
    Running,
    /// Successfully completed and verified.
    Completed,
    /// Failed execution or failed verification gate.
    Failed,
    /// Skipped due to upstream failure or bypass policy.
    Skipped,
    /// Blocked waiting for prerequisite dependencies.
    Blocked,
}

impl std::fmt::Display for GoalStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GoalStatus::Pending => write!(f, "pending"),
            GoalStatus::Running => write!(f, "running"),
            GoalStatus::Completed => write!(f, "completed"),
            GoalStatus::Failed => write!(f, "failed"),
            GoalStatus::Skipped => write!(f, "skipped"),
            GoalStatus::Blocked => write!(f, "blocked"),
        }
    }
}

impl GoalStatus {
    pub fn is_final(&self) -> bool {
        matches!(self, GoalStatus::Completed | GoalStatus::Failed | GoalStatus::Skipped)
    }

    pub fn is_successful(&self) -> bool {
        matches!(self, GoalStatus::Completed)
    }

    pub fn is_failed(&self) -> bool {
        matches!(self, GoalStatus::Failed)
    }
}

/// Categorization of relationships between goals in the DAG.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum GoalEdgeKind {
    /// Execution dependency: target node cannot start until source node completes.
    DependsOn,
    /// Hierarchical composition: child node is a sub-goal of parent node.
    SubgoalOf,
    /// Verification gate: node verifies completion and correctness of target node.
    Verifies,
    /// Rollback/mitigation action triggered upon failure of target node.
    RollbackFor,
}

impl std::fmt::Display for GoalEdgeKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GoalEdgeKind::DependsOn => write!(f, "depends_on"),
            GoalEdgeKind::SubgoalOf => write!(f, "subgoal_of"),
            GoalEdgeKind::Verifies => write!(f, "verifies"),
            GoalEdgeKind::RollbackFor => write!(f, "rollback_for"),
        }
    }
}

/// A discrete node in the hierarchical Goal DAG.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GoalNode {
    pub id: String,
    pub title: String,
    pub description: String,
    pub kind: GoalNodeKind,
    pub status: GoalStatus,
    pub estimated_duration_ms: u64,
    pub actual_duration_ms: Option<u64>,
    pub retry_count: u32,
    pub max_retries: u32,
    pub action: Option<String>,
    pub action_params: Option<serde_json::Value>,
    pub output: Option<serde_json::Value>,
    pub error: Option<String>,
    pub metadata: serde_json::Value,
}

impl GoalNode {
    pub fn new(id: impl Into<String>, title: impl Into<String>, kind: GoalNodeKind) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            description: String::new(),
            kind,
            status: GoalStatus::Pending,
            estimated_duration_ms: 1000,
            actual_duration_ms: None,
            retry_count: 0,
            max_retries: 2,
            action: None,
            action_params: None,
            output: None,
            error: None,
            metadata: serde_json::json!({}),
        }
    }

    pub fn task(id: impl Into<String>, title: impl Into<String>) -> Self {
        Self::new(id, title, GoalNodeKind::Task)
    }

    pub fn phase(id: impl Into<String>, title: impl Into<String>) -> Self {
        Self::new(id, title, GoalNodeKind::Phase)
    }

    pub fn root(id: impl Into<String>, title: impl Into<String>) -> Self {
        Self::new(id, title, GoalNodeKind::Root)
    }

    pub fn verification(id: impl Into<String>, title: impl Into<String>) -> Self {
        Self::new(id, title, GoalNodeKind::Verification)
    }

    pub fn rollback(id: impl Into<String>, title: impl Into<String>) -> Self {
        Self::new(id, title, GoalNodeKind::Rollback)
    }

    pub fn with_description(mut self, desc: impl Into<String>) -> Self {
        self.description = desc.into();
        self
    }

    pub fn with_action(mut self, action: impl Into<String>) -> Self {
        self.action = Some(action.into());
        self
    }

    pub fn with_action_params(mut self, params: serde_json::Value) -> Self {
        self.action_params = Some(params);
        self
    }

    pub fn with_max_retries(mut self, max_retries: u32) -> Self {
        self.max_retries = max_retries;
        self
    }

    pub fn with_estimated_duration(mut self, ms: u64) -> Self {
        self.estimated_duration_ms = ms;
        self
    }

    pub fn with_metadata(mut self, metadata: serde_json::Value) -> Self {
        self.metadata = metadata;
        self
    }

    pub fn mark_running(&mut self) {
        self.status = GoalStatus::Running;
    }

    pub fn mark_completed(&mut self, output: Option<serde_json::Value>, duration_ms: u64) {
        self.status = GoalStatus::Completed;
        self.output = output;
        self.actual_duration_ms = Some(duration_ms);
        self.error = None;
    }

    pub fn mark_failed(&mut self, error: impl Into<String>, duration_ms: u64) {
        self.status = GoalStatus::Failed;
        self.error = Some(error.into());
        self.actual_duration_ms = Some(duration_ms);
    }

    pub fn mark_skipped(&mut self, reason: impl Into<String>) {
        self.status = GoalStatus::Skipped;
        self.error = Some(reason.into());
    }

    pub fn mark_blocked(&mut self) {
        self.status = GoalStatus::Blocked;
    }
}

/// A directed relationship edge between goals in the planning DAG.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GoalEdge {
    pub kind: GoalEdgeKind,
    pub is_critical: bool,
    pub description: Option<String>,
}

impl GoalEdge {
    pub fn new(kind: GoalEdgeKind, is_critical: bool) -> Self {
        Self {
            kind,
            is_critical,
            description: None,
        }
    }

    pub fn depends_on() -> Self {
        Self::new(GoalEdgeKind::DependsOn, true)
    }

    pub fn non_critical_dependency() -> Self {
        Self::new(GoalEdgeKind::DependsOn, false)
    }

    pub fn subgoal_of() -> Self {
        Self::new(GoalEdgeKind::SubgoalOf, true)
    }

    pub fn verifies() -> Self {
        Self::new(GoalEdgeKind::Verifies, true)
    }

    pub fn rollback_for() -> Self {
        Self::new(GoalEdgeKind::RollbackFor, false)
    }

    pub fn with_description(mut self, desc: impl Into<String>) -> Self {
        self.description = Some(desc.into());
        self
    }
}

/// An execution wave representing an independent layer of parallelizable goals.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ExecutionWave {
    pub wave_index: usize,
    pub node_ids: Vec<String>,
    pub status: GoalStatus,
    pub duration_ms: Option<u64>,
}

impl ExecutionWave {
    pub fn new(wave_index: usize, node_ids: Vec<String>) -> Self {
        Self {
            wave_index,
            node_ids,
            status: GoalStatus::Pending,
            duration_ms: None,
        }
    }
}

/// Comprehensive report of Goal DAG execution.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DagExecutionReport {
    pub plan_id: String,
    pub root_goal: String,
    pub total_nodes: usize,
    pub completed_nodes: usize,
    pub failed_nodes: usize,
    pub skipped_nodes: usize,
    pub total_waves: usize,
    pub waves: Vec<ExecutionWave>,
    pub duration_ms: u64,
    pub success: bool,
    pub node_results: HashMap<String, GoalNode>,
    pub recovery_attempts: usize,
    pub summary: String,
}

/// Dynamic recovery action returned when a goal fails during execution.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
pub enum RecoveryAction {
    /// Retry the failed node immediately up to max retries.
    RetryNode { node_id: String, attempt: u32 },
    /// Replace the failed node with an alternate implementation.
    SubstituteNode { failed_node_id: String, replacement_node: GoalNode },
    /// Inject mitigation tasks and re-wire dependencies dynamically.
    InjectMitigation {
        failed_node_id: String,
        mitigation_nodes: Vec<GoalNode>,
        edges: Vec<(String, String, GoalEdgeKind)>,
    },
    /// Bypass the non-critical node and continue execution.
    BypassNode { failed_node_id: String, reason: String },
    /// Abort execution completely due to unrecoverable invariant violation.
    Abort { reason: String },
}
