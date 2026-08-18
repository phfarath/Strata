use std::fmt;
use std::str::FromStr;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::state::Scope;

fn default_importance() -> f32 {
    0.5
}

fn default_confidence() -> f32 {
    1.0
}

fn default_version() -> u32 {
    1
}

fn default_fact_status() -> FactStatus {
    FactStatus::Active
}

fn default_decay_d() -> f32 {
    0.5
}

/// Signal scores associated with an episodic memory recording agent affect and success.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SignalScores {
    pub success: f32,
    pub frustration: f32,
    pub novelty: f32,
    pub importance: f32,
}

impl Default for SignalScores {
    fn default() -> Self {
        Self {
            success: 1.0,
            frustration: 0.0,
            novelty: 0.5,
            importance: 0.5,
        }
    }
}

/// Episodic memory representing a consolidated episode from an agent session.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EpisodicMemory {
    pub id: Uuid,
    pub session_id: String,
    pub created_at: DateTime<Utc>,
    pub time_start: DateTime<Utc>,
    pub time_end: DateTime<Utc>,
    pub actor: String,
    #[serde(default)]
    pub project: Option<String>,
    #[serde(default)]
    pub files: Vec<String>,
    #[serde(default)]
    pub tools_used: Vec<String>,
    pub summary: String,
    #[serde(default)]
    pub goals: Vec<String>,
    #[serde(default)]
    pub obstacles: Vec<String>,
    #[serde(default)]
    pub outcomes: Vec<String>,
    #[serde(default)]
    pub signals: SignalScores,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub raw_event_ids: Vec<i64>,
}

impl EpisodicMemory {
    pub fn new(
        session_id: impl Into<String>,
        actor: impl Into<String>,
        summary: impl Into<String>,
        time_start: DateTime<Utc>,
        time_end: DateTime<Utc>,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            session_id: session_id.into(),
            created_at: Utc::now(),
            time_start,
            time_end,
            actor: actor.into(),
            project: None,
            files: Vec::new(),
            tools_used: Vec::new(),
            summary: summary.into(),
            goals: Vec::new(),
            obstacles: Vec::new(),
            outcomes: Vec::new(),
            signals: SignalScores::default(),
            tags: Vec::new(),
            raw_event_ids: Vec::new(),
        }
    }

    pub fn with_project(mut self, project: impl Into<String>) -> Self {
        self.project = Some(project.into());
        self
    }

    pub fn with_signals(mut self, signals: SignalScores) -> Self {
        self.signals = signals;
        self
    }

    pub fn with_goals(mut self, goals: Vec<String>) -> Self {
        self.goals = goals;
        self
    }

    pub fn with_obstacles(mut self, obstacles: Vec<String>) -> Self {
        self.obstacles = obstacles;
        self
    }

    pub fn with_outcomes(mut self, outcomes: Vec<String>) -> Self {
        self.outcomes = outcomes;
        self
    }

    pub fn with_tools(mut self, tools: Vec<String>) -> Self {
        self.tools_used = tools;
        self
    }

    pub fn with_files(mut self, files: Vec<String>) -> Self {
        self.files = files;
        self
    }

    pub fn with_tags(mut self, tags: Vec<String>) -> Self {
        self.tags = tags;
        self
    }

    pub fn with_raw_events(mut self, event_ids: Vec<i64>) -> Self {
        self.raw_event_ids = event_ids;
        self
    }
}

/// Truth maintenance status of a semantic fact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FactStatus {
    Active,
    Deprecated,
    Outlier,
    Candidate,
}

impl Default for FactStatus {
    fn default() -> Self {
        FactStatus::Active
    }
}

impl fmt::Display for FactStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FactStatus::Active => write!(f, "active"),
            FactStatus::Deprecated => write!(f, "deprecated"),
            FactStatus::Outlier => write!(f, "outlier"),
            FactStatus::Candidate => write!(f, "candidate"),
        }
    }
}

impl FromStr for FactStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "active" | "in" => Ok(FactStatus::Active),
            "deprecated" | "out" => Ok(FactStatus::Deprecated),
            "outlier" => Ok(FactStatus::Outlier),
            "candidate" => Ok(FactStatus::Candidate),
            _ => Err(format!("Unknown FactStatus: {s}")),
        }
    }
}

/// Reference to source evidence supporting a semantic fact.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EvidenceRef {
    pub source_type: String,
    pub source_id: String,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub event_ids: Vec<i64>,
    #[serde(default = "default_confidence")]
    pub confidence: f32,
}

impl EvidenceRef {
    pub fn new(source_type: impl Into<String>, source_id: impl Into<String>, confidence: f32) -> Self {
        Self {
            source_type: source_type.into(),
            source_id: source_id.into(),
            session_id: None,
            event_ids: Vec::new(),
            confidence: confidence.clamp(0.0, 1.0),
        }
    }

    pub fn with_session(mut self, session_id: impl Into<String>) -> Self {
        self.session_id = Some(session_id.into());
        self
    }

    pub fn with_event_ids(mut self, event_ids: Vec<i64>) -> Self {
        self.event_ids = event_ids;
        self
    }
}

/// Atomic semantic fact distilled from experience, under justification-based truth maintenance.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SemanticFact {
    pub id: Uuid,
    #[serde(default)]
    pub project: Option<String>,
    pub scope: Scope,
    pub statement: String,
    pub category: String,
    #[serde(default)]
    pub evidence: Vec<EvidenceRef>,
    #[serde(default = "default_importance")]
    pub importance: f32,
    #[serde(default = "default_confidence")]
    pub confidence: f32,
    pub created_at: DateTime<Utc>,
    pub last_updated_at: DateTime<Utc>,
    #[serde(default = "default_fact_status")]
    pub status: FactStatus,
    #[serde(default = "default_version")]
    pub version: u32,
    #[serde(default)]
    pub replaced_by: Option<Uuid>,
    #[serde(default)]
    pub tags: Vec<String>,
}

impl SemanticFact {
    pub fn new(statement: impl Into<String>, category: impl Into<String>, scope: Scope) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            project: None,
            scope,
            statement: statement.into(),
            category: category.into(),
            evidence: Vec::new(),
            importance: 0.5,
            confidence: 1.0,
            created_at: now,
            last_updated_at: now,
            status: FactStatus::Active,
            version: 1,
            replaced_by: None,
            tags: Vec::new(),
        }
    }

    pub fn with_id(mut self, id: Uuid) -> Self {
        self.id = id;
        self
    }

    pub fn with_project(mut self, project: impl Into<String>) -> Self {
        self.project = Some(project.into());
        self
    }

    pub fn with_importance(mut self, importance: f32) -> Self {

        self.importance = importance.clamp(0.0, 1.0);
        self
    }

    pub fn with_confidence(mut self, confidence: f32) -> Self {
        self.confidence = confidence.clamp(0.0, 1.0);
        self
    }

    pub fn with_evidence(mut self, evidence: Vec<EvidenceRef>) -> Self {
        self.evidence = evidence;
        self
    }

    pub fn with_tags(mut self, tags: Vec<String>) -> Self {
        self.tags = tags;
        self
    }

    pub fn deprecate_and_replace(&mut self, replacement_id: Uuid) {
        self.status = FactStatus::Deprecated;
        self.replaced_by = Some(replacement_id);
        self.last_updated_at = Utc::now();
    }
}

/// Parameter definition for a procedural skill.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParameterDef {
    pub name: String,
    pub param_type: String,
    pub description: String,
    #[serde(default)]
    pub example: Option<String>,
}

impl ParameterDef {
    pub fn new(name: impl Into<String>, param_type: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            param_type: param_type.into(),
            description: description.into(),
            example: None,
        }
    }

    pub fn with_example(mut self, example: impl Into<String>) -> Self {
        self.example = Some(example.into());
        self
    }
}

/// Individual step within a procedural skill workflow.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProceduralStep {
    pub order: u32,
    pub tool: String,
    pub action: String,
    #[serde(default)]
    pub arguments: serde_json::Value,
    #[serde(default)]
    pub expected_result: Option<String>,
    #[serde(default)]
    pub notes: Option<String>,
}

impl ProceduralStep {
    pub fn new(order: u32, tool: impl Into<String>, action: impl Into<String>, arguments: serde_json::Value) -> Self {
        Self {
            order,
            tool: tool.into(),
            action: action.into(),
            arguments,
            expected_result: None,
            notes: None,
        }
    }

    pub fn with_expected_result(mut self, expected: impl Into<String>) -> Self {
        self.expected_result = Some(expected.into());
        self
    }

    pub fn with_notes(mut self, notes: impl Into<String>) -> Self {
        self.notes = Some(notes.into());
        self
    }
}

/// Historical execution example of a procedural skill.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProceduralExample {
    pub session_id: String,
    #[serde(default)]
    pub event_ids: Vec<i64>,
    pub outcome: String,
}

impl ProceduralExample {
    pub fn new(session_id: impl Into<String>, outcome: impl Into<String>) -> Self {
        Self {
            session_id: session_id.into(),
            event_ids: Vec::new(),
            outcome: outcome.into(),
        }
    }
}

/// Consolidated procedural skill capturing reusable tool invocation patterns.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProceduralSkill {
    pub id: Uuid,
    pub name: String,
    #[serde(default)]
    pub project: Option<String>,
    pub description: String,
    #[serde(default)]
    pub preconditions: Vec<String>,
    #[serde(default)]
    pub postconditions: Vec<String>,
    #[serde(default)]
    pub parameters: Vec<ParameterDef>,
    #[serde(default)]
    pub steps: Vec<ProceduralStep>,
    #[serde(default)]
    pub examples: Vec<ProceduralExample>,
    #[serde(default = "default_confidence")]
    pub success_rate: f32,
    #[serde(default = "default_importance")]
    pub importance: f32,
    pub created_at: DateTime<Utc>,
    #[serde(default)]
    pub last_used_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub usage_count: u32,
    #[serde(default)]
    pub tags: Vec<String>,
}

impl ProceduralSkill {
    pub fn new(name: impl Into<String>, description: impl Into<String>) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            project: None,
            description: description.into(),
            preconditions: Vec::new(),
            postconditions: Vec::new(),
            parameters: Vec::new(),
            steps: Vec::new(),
            examples: Vec::new(),
            success_rate: 1.0,
            importance: 0.5,
            created_at: now,
            last_used_at: None,
            usage_count: 0,
            tags: Vec::new(),
        }
    }

    pub fn with_project(mut self, project: impl Into<String>) -> Self {
        self.project = Some(project.into());
        self
    }

    pub fn with_preconditions(mut self, preconditions: Vec<String>) -> Self {
        self.preconditions = preconditions;
        self
    }

    pub fn with_postconditions(mut self, postconditions: Vec<String>) -> Self {
        self.postconditions = postconditions;
        self
    }

    pub fn with_parameters(mut self, parameters: Vec<ParameterDef>) -> Self {
        self.parameters = parameters;
        self
    }

    pub fn with_steps(mut self, steps: Vec<ProceduralStep>) -> Self {
        self.steps = steps;
        self
    }

    pub fn with_examples(mut self, examples: Vec<ProceduralExample>) -> Self {
        self.examples = examples;
        self
    }

    pub fn with_tags(mut self, tags: Vec<String>) -> Self {
        self.tags = tags;
        self
    }

    pub fn record_usage(&mut self, success: bool) {
        self.usage_count += 1;
        self.last_used_at = Some(Utc::now());
        let alpha = 0.2;
        let outcome_val = if success { 1.0 } else { 0.0 };
        self.success_rate = (1.0 - alpha) * self.success_rate + alpha * outcome_val;
    }
}

/// Configuration parameters for ACT-R and Ebbinghaus memory decay functions.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DecayConfig {
    /// Scaling weight for frequency-based power-law activation component
    pub alpha: f32,
    /// Weight for intrinsic memory importance component
    pub beta: f32,
    /// Weight for confidence component
    pub gamma: f32,
    /// Power-law time decay exponent (typically 0.5)
    #[serde(default = "default_decay_d")]
    pub d: f32,
    /// Base memory stability in hours
    pub s0: f32,
    /// Stability growth coefficient per logarithmic access count
    pub lambda: f32,
    /// Stability boost factor per unit importance
    pub mu: f32,
    /// Retention threshold below which memories are eligible for pruning
    pub prune_threshold: f32,
    /// Importance threshold at and above which memories are permanent invariants
    pub invariant_threshold: f32,
}

impl Default for DecayConfig {
    fn default() -> Self {
        Self {
            alpha: 1.0,
            beta: 0.5,
            gamma: 0.5,
            d: 0.5,
            s0: 24.0,
            lambda: 0.1,
            mu: 0.2,
            prune_threshold: 0.05,
            invariant_threshold: 0.95,
        }
    }
}

/// Calculated metrics from evaluating memory decay models.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DecayMetrics {
    /// ACT-R base-level activation score
    pub activation: f32,
    /// Ebbinghaus retention probability [0.0, 1.0]
    pub retention: f32,
    /// Current stability in hours
    pub stability: f32,
    /// Whether retention is below the prune threshold
    pub is_expired: bool,
}

fn default_batch_size() -> usize {
    100
}

fn default_max_retries() -> u32 {
    5
}

fn default_base_backoff_ms() -> u64 {
    500
}

/// Represents a change data capture (CDC) delta for offline-first synchronization.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SyncDelta {
    pub id: Uuid,
    pub workspace_id: String,
    pub seq: u64,
    pub ts: DateTime<Utc>,
    pub kind: String,
    pub payload: serde_json::Value,
    pub version_hash: String,
    #[serde(default)]
    pub synced: bool,
}

impl SyncDelta {
    pub fn new(
        workspace_id: impl Into<String>,
        seq: u64,
        kind: impl Into<String>,
        payload: serde_json::Value,
        version_hash: impl Into<String>,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            workspace_id: workspace_id.into(),
            seq,
            ts: Utc::now(),
            kind: kind.into(),
            payload,
            version_hash: version_hash.into(),
            synced: false,
        }
    }

    pub fn with_id(mut self, id: Uuid) -> Self {
        self.id = id;
        self
    }

    pub fn with_ts(mut self, ts: DateTime<Utc>) -> Self {
        self.ts = ts;
        self
    }

    pub fn with_synced(mut self, synced: bool) -> Self {
        self.synced = synced;
        self
    }
}

/// User or agent feedback on a retrieved memory.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemoryFeedback {
    pub memory_id: Uuid,
    pub rating: String,
    #[serde(default)]
    pub score: Option<f32>,
    #[serde(default)]
    pub comment: Option<String>,
    pub created_at: DateTime<Utc>,
}

impl MemoryFeedback {
    pub fn new(memory_id: Uuid, rating: impl Into<String>) -> Self {
        Self {
            memory_id,
            rating: rating.into(),
            score: None,
            comment: None,
            created_at: Utc::now(),
        }
    }

    pub fn positive(memory_id: Uuid) -> Self {
        Self {
            memory_id,
            rating: "positive".to_string(),
            score: Some(1.0),
            comment: None,
            created_at: Utc::now(),
        }
    }

    pub fn negative(memory_id: Uuid, comment: Option<String>) -> Self {
        Self {
            memory_id,
            rating: "negative".to_string(),
            score: Some(1.0),
            comment,
            created_at: Utc::now(),
        }
    }


    pub fn with_score(mut self, score: f32) -> Self {
        self.score = Some(score.clamp(0.0, 1.0));
        self
    }

    pub fn with_comment(mut self, comment: impl Into<String>) -> Self {
        self.comment = Some(comment.into());
        self
    }

    pub fn with_created_at(mut self, created_at: DateTime<Utc>) -> Self {
        self.created_at = created_at;
        self
    }
}

/// Configuration for the synchronization engine.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SyncConfig {
    #[serde(default)]
    pub endpoint: Option<String>,
    #[serde(default)]
    pub token: Option<String>,
    pub workspace_id: String,
    #[serde(default = "default_batch_size")]
    pub batch_size: usize,
    #[serde(default = "default_max_retries")]
    pub max_retries: u32,
    #[serde(default = "default_base_backoff_ms")]
    pub base_backoff_ms: u64,
}

impl SyncConfig {
    pub fn new(workspace_id: impl Into<String>) -> Self {
        Self {
            endpoint: None,
            token: None,
            workspace_id: workspace_id.into(),
            batch_size: 100,
            max_retries: 5,
            base_backoff_ms: 500,
        }
    }

    pub fn with_endpoint(mut self, endpoint: impl Into<String>) -> Self {
        self.endpoint = Some(endpoint.into());
        self
    }

    pub fn with_token(mut self, token: impl Into<String>) -> Self {
        self.token = Some(token.into());
        self
    }

    pub fn with_batch_size(mut self, batch_size: usize) -> Self {
        self.batch_size = batch_size;
        self
    }

    pub fn with_max_retries(mut self, max_retries: u32) -> Self {
        self.max_retries = max_retries;
        self
    }

    pub fn with_base_backoff_ms(mut self, base_backoff_ms: u64) -> Self {
        self.base_backoff_ms = base_backoff_ms;
        self
    }
}

impl Default for SyncConfig {
    fn default() -> Self {
        Self {
            endpoint: None,
            token: None,
            workspace_id: "default".to_string(),
            batch_size: 100,
            max_retries: 5,
            base_backoff_ms: 500,
        }
    }
}

/// Summary report of a sync cycle execution.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct SyncReport {
    pub pushed_count: usize,
    pub pulled_count: usize,
    pub conflicts_resolved: usize,
    pub last_seq: u64,
    pub errors: Vec<String>,
}
