use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;
use uuid::Uuid;

use crate::state::{MemoryTier, Scope};

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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum FactStatus {
    #[default]
    Active,
    Deprecated,
    Outlier,
    Candidate,
    Stale,
    Suspicious,
}

impl fmt::Display for FactStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FactStatus::Active => write!(f, "active"),
            FactStatus::Deprecated => write!(f, "deprecated"),
            FactStatus::Outlier => write!(f, "outlier"),
            FactStatus::Candidate => write!(f, "candidate"),
            FactStatus::Stale => write!(f, "stale"),
            FactStatus::Suspicious => write!(f, "suspicious"),
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
            "stale" => Ok(FactStatus::Stale),
            "suspicious" | "suspect" => Ok(FactStatus::Suspicious),
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
    pub fn new(
        source_type: impl Into<String>,
        source_id: impl Into<String>,
        confidence: f32,
    ) -> Self {
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
    #[serde(default)]
    pub tier: MemoryTier,
    #[serde(default)]
    pub approved_by_human: bool,
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
    #[serde(default)]
    pub code_anchor: Option<CodeAnchor>,
    #[serde(default)]
    pub depends_on: Vec<Uuid>,
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
            tier: MemoryTier::Peripheral,
            approved_by_human: false,
            created_at: now,
            last_updated_at: now,
            status: FactStatus::Active,
            version: 1,
            replaced_by: None,
            tags: Vec::new(),
            code_anchor: None,
            depends_on: Vec::new(),
        }
    }

    pub fn with_depends_on(mut self, depends_on: Vec<Uuid>) -> Self {
        self.depends_on = depends_on;
        self
    }

    pub fn with_dependency(mut self, prerequisite_id: Uuid) -> Self {
        if !self.depends_on.contains(&prerequisite_id) {
            self.depends_on.push(prerequisite_id);
        }
        self
    }

    pub fn add_dependency(&mut self, prerequisite_id: Uuid) {
        if !self.depends_on.contains(&prerequisite_id) {
            self.depends_on.push(prerequisite_id);
        }
    }

    pub fn with_tier(mut self, tier: MemoryTier) -> Self {
        self.tier = tier;
        self
    }

    pub fn with_human_approval(mut self, approved: bool) -> Self {
        self.approved_by_human = approved;
        self
    }

    pub fn is_approved_by_human(&self) -> bool {
        self.approved_by_human
    }

    pub fn is_core(&self) -> bool {
        self.tier == MemoryTier::Core
    }

    pub fn is_working(&self) -> bool {
        self.tier == MemoryTier::Working
    }

    pub fn is_peripheral(&self) -> bool {
        self.tier == MemoryTier::Peripheral
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

    pub fn with_code_anchor(mut self, anchor: CodeAnchor) -> Self {
        self.code_anchor = Some(anchor);
        self
    }

    pub fn deprecate_and_replace(&mut self, replacement_id: Uuid) {
        self.status = FactStatus::Deprecated;
        self.replaced_by = Some(replacement_id);
        self.last_updated_at = Utc::now();
        if let Some(ref mut anchor) = self.code_anchor {
            anchor.invalidate();
        }
    }

    pub fn mark_stale(&mut self) {
        self.status = FactStatus::Stale;
        self.last_updated_at = Utc::now();
        if let Some(ref mut anchor) = self.code_anchor {
            anchor.invalidate();
        }
    }

    pub fn mark_suspicious(&mut self) {
        self.status = FactStatus::Suspicious;
        self.last_updated_at = Utc::now();
    }

    pub fn is_stale(&self) -> bool {
        self.status == FactStatus::Stale
    }

    pub fn is_suspicious(&self) -> bool {
        self.status == FactStatus::Suspicious
    }

    pub fn is_active(&self) -> bool {
        self.status == FactStatus::Active
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
    pub fn new(
        name: impl Into<String>,
        param_type: impl Into<String>,
        description: impl Into<String>,
    ) -> Self {
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
    pub fn new(
        order: u32,
        tool: impl Into<String>,
        action: impl Into<String>,
        arguments: serde_json::Value,
    ) -> Self {
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

/// Categorization of implicit behavioural signals observed during execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SignalKind {
    ToolLoop,
    GitRevert,
    CommandFix,
    TestRerunFail,
    TestRerunSuccess,
    LongDwell,
    ExplicitRating,
    ExplicitComment,
}

impl fmt::Display for SignalKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SignalKind::ToolLoop => write!(f, "tool_loop"),
            SignalKind::GitRevert => write!(f, "git_revert"),
            SignalKind::CommandFix => write!(f, "command_fix"),
            SignalKind::TestRerunFail => write!(f, "test_rerun_fail"),
            SignalKind::TestRerunSuccess => write!(f, "test_rerun_success"),
            SignalKind::LongDwell => write!(f, "long_dwell"),
            SignalKind::ExplicitRating => write!(f, "explicit_rating"),
            SignalKind::ExplicitComment => write!(f, "explicit_comment"),
        }
    }
}

impl FromStr for SignalKind {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "tool_loop" | "toolloop" => Ok(SignalKind::ToolLoop),
            "git_revert" | "gitrevert" => Ok(SignalKind::GitRevert),
            "command_fix" | "commandfix" => Ok(SignalKind::CommandFix),
            "test_rerun_fail" | "testrerunfail" => Ok(SignalKind::TestRerunFail),
            "test_rerun_success" | "testrerunsuccess" => Ok(SignalKind::TestRerunSuccess),
            "long_dwell" | "longdwell" => Ok(SignalKind::LongDwell),
            "explicit_rating" | "explicitrating" => Ok(SignalKind::ExplicitRating),
            "explicit_comment" | "explicitcomment" => Ok(SignalKind::ExplicitComment),
            _ => Err(format!("Unknown SignalKind: {s}")),
        }
    }
}

/// Implicit behavioural signal captured during agent run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImplicitSignal {
    pub id: Uuid,
    pub kind: SignalKind,
    pub timestamp: DateTime<Utc>,
    pub session_id: String,
    pub agent_id: String,
    #[serde(default)]
    pub tool_name: Option<String>,
    #[serde(default)]
    pub file_path: Option<String>,
    #[serde(default)]
    pub extra: Option<String>,
}

impl ImplicitSignal {
    pub fn new(
        kind: SignalKind,
        session_id: impl Into<String>,
        agent_id: impl Into<String>,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            kind,
            timestamp: Utc::now(),
            session_id: session_id.into(),
            agent_id: agent_id.into(),
            tool_name: None,
            file_path: None,
            extra: None,
        }
    }

    pub fn with_id(mut self, id: Uuid) -> Self {
        self.id = id;
        self
    }

    pub fn with_timestamp(mut self, timestamp: DateTime<Utc>) -> Self {
        self.timestamp = timestamp;
        self
    }

    pub fn with_tool_name(mut self, tool_name: impl Into<String>) -> Self {
        self.tool_name = Some(tool_name.into());
        self
    }

    pub fn with_file_path(mut self, file_path: impl Into<String>) -> Self {
        self.file_path = Some(file_path.into());
        self
    }

    pub fn with_extra(mut self, extra: impl Into<String>) -> Self {
        self.extra = Some(extra.into());
        self
    }
}

/// Qualitative rating for feedback.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FeedbackRating {
    Positive,
    Negative,
}

impl fmt::Display for FeedbackRating {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FeedbackRating::Positive => write!(f, "positive"),
            FeedbackRating::Negative => write!(f, "negative"),
        }
    }
}

impl FromStr for FeedbackRating {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "positive" | "pos" | "+" | "1" => Ok(FeedbackRating::Positive),
            "negative" | "neg" | "-" | "0" | "-1" => Ok(FeedbackRating::Negative),
            _ => Err(format!("Unknown FeedbackRating: {s}")),
        }
    }
}

/// Fine-grained feedback event recorded against a memory or behavioural signal.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FeedbackEvent {
    pub id: Uuid,
    #[serde(default)]
    pub memory_id: Option<Uuid>,
    #[serde(default)]
    pub signal_id: Option<Uuid>,
    pub rating: FeedbackRating,
    #[serde(default)]
    pub comment: Option<String>,
    pub timestamp: DateTime<Utc>,
    pub source: String,
}

impl FeedbackEvent {
    pub fn new(rating: FeedbackRating, source: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            memory_id: None,
            signal_id: None,
            rating,
            comment: None,
            timestamp: Utc::now(),
            source: source.into(),
        }
    }

    pub fn with_id(mut self, id: Uuid) -> Self {
        self.id = id;
        self
    }

    pub fn with_memory_id(mut self, memory_id: Uuid) -> Self {
        self.memory_id = Some(memory_id);
        self
    }

    pub fn with_signal_id(mut self, signal_id: Uuid) -> Self {
        self.signal_id = Some(signal_id);
        self
    }

    pub fn with_comment(mut self, comment: impl Into<String>) -> Self {
        self.comment = Some(comment.into());
        self
    }

    pub fn with_timestamp(mut self, timestamp: DateTime<Utc>) -> Self {
        self.timestamp = timestamp;
        self
    }
}

/// DPO preference pair for alignment tuning.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PreferencePair {
    pub id: Uuid,
    pub prompt: String,
    pub chosen: String,
    pub rejected: String,
    pub source_session_id: String,
    pub created_at: DateTime<Utc>,
    #[serde(default)]
    pub oracle_verified: bool,
    #[serde(default)]
    pub verification_source: Option<String>,
}

impl PreferencePair {
    pub fn new(
        prompt: impl Into<String>,
        chosen: impl Into<String>,
        rejected: impl Into<String>,
        source_session_id: impl Into<String>,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            prompt: prompt.into(),
            chosen: chosen.into(),
            rejected: rejected.into(),
            source_session_id: source_session_id.into(),
            created_at: Utc::now(),
            oracle_verified: false,
            verification_source: None,
        }
    }

    pub fn with_id(mut self, id: Uuid) -> Self {
        self.id = id;
        self
    }

    pub fn with_created_at(mut self, created_at: DateTime<Utc>) -> Self {
        self.created_at = created_at;
        self
    }

    pub fn with_oracle_verified(mut self, verified: bool) -> Self {
        self.oracle_verified = verified;
        self
    }

    pub fn with_verification_source(mut self, source: impl Into<String>) -> Self {
        self.verification_source = Some(source.into());
        self
    }

    pub fn with_verification(mut self, verified: bool, source: Option<String>) -> Self {
        self.oracle_verified = verified;
        self.verification_source = source;
        self
    }
}

/// KTO (Kahneman-Tversky Optimization) sample with binary outcome label.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KtoSample {
    pub id: Uuid,
    pub prompt: String,
    pub completion: String,
    pub label: bool,
    pub source_session_id: String,
    pub created_at: DateTime<Utc>,
    #[serde(default)]
    pub oracle_verified: bool,
    #[serde(default)]
    pub verification_source: Option<String>,
}

impl KtoSample {
    pub fn new(
        prompt: impl Into<String>,
        completion: impl Into<String>,
        label: bool,
        source_session_id: impl Into<String>,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            prompt: prompt.into(),
            completion: completion.into(),
            label,
            source_session_id: source_session_id.into(),
            created_at: Utc::now(),
            oracle_verified: false,
            verification_source: None,
        }
    }

    pub fn with_id(mut self, id: Uuid) -> Self {
        self.id = id;
        self
    }

    pub fn with_created_at(mut self, created_at: DateTime<Utc>) -> Self {
        self.created_at = created_at;
        self
    }

    pub fn with_oracle_verified(mut self, verified: bool) -> Self {
        self.oracle_verified = verified;
        self
    }

    pub fn with_verification_source(mut self, source: impl Into<String>) -> Self {
        self.verification_source = Some(source.into());
        self
    }

    pub fn with_verification(mut self, verified: bool, source: Option<String>) -> Self {
        self.oracle_verified = verified;
        self.verification_source = source;
        self
    }
}

/// Supervised Fine-Tuning (SFT) sample formatted as instruction-input-output triple.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SftSample {
    pub id: Uuid,
    pub instruction: String,
    pub input: String,
    pub output: String,
    pub source_session_id: String,
    pub created_at: DateTime<Utc>,
    #[serde(default)]
    pub oracle_verified: bool,
    #[serde(default)]
    pub verification_source: Option<String>,
}

impl SftSample {
    pub fn new(
        instruction: impl Into<String>,
        input: impl Into<String>,
        output: impl Into<String>,
        source_session_id: impl Into<String>,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            instruction: instruction.into(),
            input: input.into(),
            output: output.into(),
            source_session_id: source_session_id.into(),
            created_at: Utc::now(),
            oracle_verified: false,
            verification_source: None,
        }
    }

    pub fn with_id(mut self, id: Uuid) -> Self {
        self.id = id;
        self
    }

    pub fn with_created_at(mut self, created_at: DateTime<Utc>) -> Self {
        self.created_at = created_at;
        self
    }

    pub fn with_oracle_verified(mut self, verified: bool) -> Self {
        self.oracle_verified = verified;
        self
    }

    pub fn with_verification_source(mut self, source: impl Into<String>) -> Self {
        self.verification_source = Some(source.into());
        self
    }

    pub fn with_verification(mut self, verified: bool, source: Option<String>) -> Self {
        self.oracle_verified = verified;
        self.verification_source = source;
        self
    }
}

/// Supported export formats for alignment datasets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExportFormat {
    Dpo,
    Kto,
    Sft,
    Markdown,
    Jsonl,
}

impl fmt::Display for ExportFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ExportFormat::Dpo => write!(f, "dpo"),
            ExportFormat::Kto => write!(f, "kto"),
            ExportFormat::Sft => write!(f, "sft"),
            ExportFormat::Markdown => write!(f, "markdown"),
            ExportFormat::Jsonl => write!(f, "jsonl"),
        }
    }
}

impl FromStr for ExportFormat {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "dpo" => Ok(ExportFormat::Dpo),
            "kto" => Ok(ExportFormat::Kto),
            "sft" => Ok(ExportFormat::Sft),
            "markdown" | "md" => Ok(ExportFormat::Markdown),
            "jsonl" | "json_lines" => Ok(ExportFormat::Jsonl),
            _ => Err(format!("Unknown ExportFormat: {s}")),
        }
    }
}

/// Budget and content configuration for context compilation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContextBudgetConfig {
    pub max_tokens: usize,
    pub top_k_memories: usize,
    pub include_failure_patterns: bool,
    pub include_success_trajectories: bool,
}

impl Default for ContextBudgetConfig {
    fn default() -> Self {
        Self {
            max_tokens: 2048,
            top_k_memories: 10,
            include_failure_patterns: true,
            include_success_trajectories: true,
        }
    }
}

impl ContextBudgetConfig {
    pub fn new(max_tokens: usize, top_k_memories: usize) -> Self {
        Self {
            max_tokens,
            top_k_memories,
            include_failure_patterns: true,
            include_success_trajectories: true,
        }
    }

    pub fn with_failure_patterns(mut self, include: bool) -> Self {
        self.include_failure_patterns = include;
        self
    }

    pub fn with_success_trajectories(mut self, include: bool) -> Self {
        self.include_success_trajectories = include;
        self
    }
}

/// Host target configuration for multi-IDE context distribution.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HostTargetConfig {
    pub cursor: bool,
    pub claude: bool,
    pub codex: bool,
    pub gemini: bool,
}

impl Default for HostTargetConfig {
    fn default() -> Self {
        Self {
            cursor: true,
            claude: true,
            codex: true,
            gemini: true,
        }
    }
}

impl HostTargetConfig {
    pub fn all() -> Self {
        Self::default()
    }

    pub fn none() -> Self {
        Self {
            cursor: false,
            claude: false,
            codex: false,
            gemini: false,
        }
    }
}

fn default_true() -> bool {
    true
}

/// Type of code symbol extracted from AST.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SymbolType {
    Function,
    Method,
    Struct,
    Class,
    Interface,
    Trait,
    Enum,
    TypeAlias,
    Module,
    Constant,
    Variable,
    Other,
}

impl fmt::Display for SymbolType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SymbolType::Function => write!(f, "function"),
            SymbolType::Method => write!(f, "method"),
            SymbolType::Struct => write!(f, "struct"),
            SymbolType::Class => write!(f, "class"),
            SymbolType::Interface => write!(f, "interface"),
            SymbolType::Trait => write!(f, "trait"),
            SymbolType::Enum => write!(f, "enum"),
            SymbolType::TypeAlias => write!(f, "type_alias"),
            SymbolType::Module => write!(f, "module"),
            SymbolType::Constant => write!(f, "constant"),
            SymbolType::Variable => write!(f, "variable"),
            SymbolType::Other => write!(f, "other"),
        }
    }
}

impl FromStr for SymbolType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "function" | "fn" => Ok(SymbolType::Function),
            "method" => Ok(SymbolType::Method),
            "struct" => Ok(SymbolType::Struct),
            "class" => Ok(SymbolType::Class),
            "interface" => Ok(SymbolType::Interface),
            "trait" => Ok(SymbolType::Trait),
            "enum" => Ok(SymbolType::Enum),
            "type_alias" | "type" => Ok(SymbolType::TypeAlias),
            "module" | "mod" => Ok(SymbolType::Module),
            "constant" | "const" => Ok(SymbolType::Constant),
            "variable" | "var" | "let" => Ok(SymbolType::Variable),
            "other" => Ok(SymbolType::Other),
            _ => Err(format!("Unknown SymbolType: {s}")),
        }
    }
}

/// Structural AST anchor pinning a memory to a code symbol with bi-temporal validity.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CodeAnchor {
    pub file_path: String,
    pub symbol_path: String,
    pub symbol_type: SymbolType,
    #[serde(default)]
    pub git_commit_hash: Option<String>,
    pub ast_node_hash: String,
    #[serde(default)]
    pub content_hash: Option<String>,
    pub start_line: u32,
    pub end_line: u32,
    pub valid_from: DateTime<Utc>,
    #[serde(default)]
    pub valid_until: Option<DateTime<Utc>>,
    #[serde(default = "default_true")]
    pub is_valid: bool,
}

impl CodeAnchor {
    pub fn new(
        file_path: impl Into<String>,
        symbol_path: impl Into<String>,
        symbol_type: SymbolType,
        ast_node_hash: impl Into<String>,
        start_line: u32,
        end_line: u32,
    ) -> Self {
        Self {
            file_path: file_path.into(),
            symbol_path: symbol_path.into(),
            symbol_type,
            git_commit_hash: None,
            ast_node_hash: ast_node_hash.into(),
            content_hash: None,
            start_line,
            end_line,
            valid_from: Utc::now(),
            valid_until: None,
            is_valid: true,
        }
    }

    pub fn with_content_hash(mut self, hash: impl Into<String>) -> Self {
        self.content_hash = Some(hash.into());
        self
    }

    pub fn with_git_commit(mut self, commit_hash: impl Into<String>) -> Self {
        self.git_commit_hash = Some(commit_hash.into());
        self
    }

    pub fn with_validity(
        mut self,
        valid_from: DateTime<Utc>,
        valid_until: Option<DateTime<Utc>>,
    ) -> Self {
        let is_valid = match valid_until {
            Some(until) => until > Utc::now(),
            None => true,
        };
        self.valid_from = valid_from;
        self.valid_until = valid_until;
        self.is_valid = is_valid;
        self
    }

    pub fn invalidate(&mut self) {
        self.valid_until = Some(Utc::now());
        self.is_valid = false;
    }

    pub fn is_active_at(&self, time: DateTime<Utc>) -> bool {
        if !self.is_valid {
            return false;
        }
        if time < self.valid_from {
            return false;
        }
        if let Some(until) = self.valid_until {
            time <= until
        } else {
            true
        }
    }

    pub fn is_active(&self) -> bool {
        self.is_active_at(Utc::now())
    }
}

/// Audit record capturing every JTMS belief revision and contradiction resolution event.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JtmsAuditRow {
    pub id: Uuid,
    pub winning_fact_id: Uuid,
    pub losing_fact_id: Uuid,
    pub resolution_type: String,
    pub reason: String,
    #[serde(default)]
    pub contradiction_cues: Vec<String>,
    #[serde(default)]
    pub similarity: f32,
    pub timestamp: DateTime<Utc>,
    #[serde(default)]
    pub metadata: serde_json::Value,
}

impl JtmsAuditRow {
    pub fn new(
        winning_fact_id: Uuid,
        losing_fact_id: Uuid,
        resolution_type: impl Into<String>,
        reason: impl Into<String>,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            winning_fact_id,
            losing_fact_id,
            resolution_type: resolution_type.into(),
            reason: reason.into(),
            contradiction_cues: Vec::new(),
            similarity: 0.0,
            timestamp: Utc::now(),
            metadata: serde_json::json!({}),
        }
    }

    pub fn with_cues(mut self, cues: Vec<String>) -> Self {
        self.contradiction_cues = cues;
        self
    }

    pub fn with_similarity(mut self, similarity: f32) -> Self {
        self.similarity = similarity;
        self
    }

    pub fn with_metadata(mut self, metadata: serde_json::Value) -> Self {
        self.metadata = metadata;
        self
    }
}

/// Explicit dependency edge between semantic facts in the JTMS belief graph.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FactDependency {
    pub id: Uuid,
    pub dependent_fact_id: Uuid,
    pub prerequisite_fact_id: Uuid,
    pub dependency_type: String,
    pub created_at: DateTime<Utc>,
}

impl FactDependency {
    pub fn new(
        dependent_fact_id: Uuid,
        prerequisite_fact_id: Uuid,
        dependency_type: impl Into<String>,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            dependent_fact_id,
            prerequisite_fact_id,
            dependency_type: dependency_type.into(),
            created_at: Utc::now(),
        }
    }
}
