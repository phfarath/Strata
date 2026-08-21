use std::fmt;
use std::str::FromStr;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MemoryType {
    Episodic,
    Semantic,
    Procedural,
    #[serde(alias = "KnownFailure")]
    NegativePattern,
}

impl fmt::Display for MemoryType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MemoryType::Episodic => write!(f, "Episodic"),
            MemoryType::Semantic => write!(f, "Semantic"),
            MemoryType::Procedural => write!(f, "Procedural"),
            MemoryType::NegativePattern => write!(f, "NegativePattern"),
        }
    }
}

impl FromStr for MemoryType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "episodic" => Ok(MemoryType::Episodic),
            "semantic" => Ok(MemoryType::Semantic),
            "procedural" => Ok(MemoryType::Procedural),
            "negativepattern" | "negative_pattern" | "knownfailure" | "known_failure" => {
                Ok(MemoryType::NegativePattern)
            }
            _ => Err(format!("Unknown memory type: {s}")),
        }
    }
}

/// Formal cognitive tier defining retention dynamics and budgeting for memory.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryTier {
    /// Invariant rules, critical security constraints, project axioms. Frozen decay (R=1.0), never pruned.
    Core,
    /// Active task session context, touched files, immediate diffs. FIFO + task saliency.
    Working,
    /// Historical facts, conversation context, intermediate decisions. Exponential mathematical decay -> Cold Storage.
    Peripheral,
}

impl MemoryTier {
    pub fn is_core(&self) -> bool {
        matches!(self, MemoryTier::Core)
    }

    pub fn is_working(&self) -> bool {
        matches!(self, MemoryTier::Working)
    }

    pub fn is_peripheral(&self) -> bool {
        matches!(self, MemoryTier::Peripheral)
    }
}

impl Default for MemoryTier {
    fn default() -> Self {
        MemoryTier::Peripheral
    }
}

impl fmt::Display for MemoryTier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MemoryTier::Core => write!(f, "Core"),
            MemoryTier::Working => write!(f, "Working"),
            MemoryTier::Peripheral => write!(f, "Peripheral"),
        }
    }
}

impl FromStr for MemoryTier {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "core" => Ok(MemoryTier::Core),
            "working" => Ok(MemoryTier::Working),
            "peripheral" => Ok(MemoryTier::Peripheral),
            _ => Err(format!("Unknown memory tier: {s}")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Scope {
    Global,
    Session(String),
    Project(String),
    Organization(String),
    User(String),
}

impl fmt::Display for Scope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Scope::Global => write!(f, "global"),
            Scope::Session(id) => write!(f, "session:{}", id),
            Scope::Project(id) => write!(f, "project:{}", id),
            Scope::Organization(id) => write!(f, "org:{}", id),
            Scope::User(id) => write!(f, "user:{}", id),
        }
    }
}

impl FromStr for Scope {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.eq_ignore_ascii_case("global") {
            return Ok(Scope::Global);
        }
        if let Some(rest) = s.strip_prefix("session:") {
            return Ok(Scope::Session(rest.to_string()));
        }
        if let Some(rest) = s.strip_prefix("project:") {
            return Ok(Scope::Project(rest.to_string()));
        }
        if let Some(rest) = s.strip_prefix("org:") {
            return Ok(Scope::Organization(rest.to_string()));
        }
        if let Some(rest) = s.strip_prefix("user:") {
            return Ok(Scope::User(rest.to_string()));
        }
        Ok(Scope::Project(s.to_string()))
    }
}

impl Scope {
    pub fn is_compatible(&self, target: &Scope) -> bool {
        match (self, target) {
            (Scope::Global, _) | (_, Scope::Global) => true,
            (Scope::Session(a), Scope::Session(b)) => a == b,
            (Scope::Project(a), Scope::Project(b)) => a == b,
            (Scope::Organization(a), Scope::Organization(b)) => a == b,
            (Scope::User(a), Scope::User(b)) => a == b,
            _ => false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemoryRecord {
    pub id: Uuid,
    pub memory_type: MemoryType,
    pub content: String,
    pub summary: Option<String>,
    pub scope: Scope,
    #[serde(default)]
    pub tier: MemoryTier,
    #[serde(default)]
    pub approved_by_human: bool,
    pub importance: f32,
    pub confidence: f32,
    #[serde(default)]
    pub tags: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub access_count: u64,
    pub last_accessed_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub evidence_ids: Vec<Uuid>,
    pub embedding: Option<Vec<f32>>,
    #[serde(default)]
    pub metadata: serde_json::Value,
}

impl MemoryRecord {
    pub fn new(memory_type: MemoryType, content: impl Into<String>, scope: Scope) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            memory_type,
            content: content.into(),
            summary: None,
            scope,
            tier: MemoryTier::Peripheral,
            approved_by_human: false,
            importance: 0.5,
            confidence: 1.0,
            tags: Vec::new(),
            created_at: now,
            updated_at: now,
            access_count: 0,
            last_accessed_at: None,
            evidence_ids: Vec::new(),
            embedding: None,
            metadata: serde_json::Value::Null,
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

    pub fn with_summary(mut self, summary: impl Into<String>) -> Self {
        self.summary = Some(summary.into());
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

    pub fn with_tags(mut self, tags: Vec<String>) -> Self {
        self.tags = tags;
        self
    }

    pub fn with_embedding(mut self, embedding: Vec<f32>) -> Self {
        self.embedding = Some(embedding);
        self
    }

    pub fn with_metadata(mut self, metadata: serde_json::Value) -> Self {
        self.metadata = metadata;
        self
    }

    pub fn mark_accessed(&mut self) {
        self.access_count += 1;
        self.last_accessed_at = Some(Utc::now());
    }

    pub fn to_handle(&self, score: Option<f32>) -> MemoryHandle {
        let title = self
            .summary
            .clone()
            .unwrap_or_else(|| {
                let lines = self.content.lines().next().unwrap_or("Untitled");
                if lines.len() > 60 {
                    format!("{}...", &lines[..57])
                } else {
                    lines.to_string()
                }
            });
        let summary = self
            .summary
            .clone()
            .unwrap_or_else(|| {
                if self.content.len() > 140 {
                    format!("{}...", &self.content[..137])
                } else {
                    self.content.clone()
                }
            });
        MemoryHandle {
            id: self.id,
            title,
            summary,
            memory_type: self.memory_type.clone(),
            scope: self.scope.clone(),
            tier: self.tier,
            approved_by_human: self.approved_by_human,
            relevance_score: score,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemoryHandle {
    pub id: Uuid,
    pub title: String,
    pub summary: String,
    pub memory_type: MemoryType,
    pub scope: Scope,
    #[serde(default)]
    pub tier: MemoryTier,
    #[serde(default)]
    pub approved_by_human: bool,
    pub relevance_score: Option<f32>,
}

impl MemoryHandle {
    pub fn is_core(&self) -> bool {
        self.tier.is_core()
    }

    pub fn is_working(&self) -> bool {
        self.tier.is_working()
    }

    pub fn is_peripheral(&self) -> bool {
        self.tier.is_peripheral()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SessionStatus {
    Active,
    Completed,
    Failed,
    Paused,
}

impl Default for SessionStatus {
    fn default() -> Self {
        Self::Active
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionState {
    pub session_id: String,
    pub agent_id: String,
    pub status: SessionStatus,
    #[serde(default)]
    pub working_memory: serde_json::Value,
    pub active_task: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(default)]
    pub metadata: serde_json::Value,
}

impl SessionState {
    pub fn new(session_id: impl Into<String>, agent_id: impl Into<String>) -> Self {
        let now = Utc::now();
        Self {
            session_id: session_id.into(),
            agent_id: agent_id.into(),
            status: SessionStatus::Active,
            working_memory: serde_json::json!({}),
            active_task: None,
            created_at: now,
            updated_at: now,
            metadata: serde_json::Value::Null,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum FailureSeverity {
    Low,
    Medium,
    High,
    Critical,
}

impl Default for FailureSeverity {
    fn default() -> Self {
        Self::Medium
    }
}

impl fmt::Display for FailureSeverity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FailureSeverity::Low => write!(f, "low"),
            FailureSeverity::Medium => write!(f, "medium"),
            FailureSeverity::High => write!(f, "high"),
            FailureSeverity::Critical => write!(f, "critical"),
        }
    }
}

impl FromStr for FailureSeverity {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "low" => Ok(FailureSeverity::Low),
            "high" => Ok(FailureSeverity::High),
            "critical" => Ok(FailureSeverity::Critical),
            _ => Ok(FailureSeverity::Medium),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FailurePattern {
    pub id: Uuid,
    pub signature: String,
    pub pattern_name: String,
    pub description: String,
    pub trigger_condition: String,
    pub error_type: String,
    pub mitigation: String,
    pub occurrences: u64,
    pub first_seen: DateTime<Utc>,
    pub last_seen: DateTime<Utc>,
    pub severity: FailureSeverity,
    pub scope: Scope,
    #[serde(default)]
    pub metadata: serde_json::Value,
}

impl FailurePattern {
    pub fn new(
        signature: impl Into<String>,
        pattern_name: impl Into<String>,
        description: impl Into<String>,
        mitigation: impl Into<String>,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            signature: signature.into(),
            pattern_name: pattern_name.into(),
            description: description.into(),
            trigger_condition: String::new(),
            error_type: "GenericError".to_string(),
            mitigation: mitigation.into(),
            occurrences: 1,
            first_seen: now,
            last_seen: now,
            severity: FailureSeverity::Medium,
            scope: Scope::Global,
            metadata: serde_json::Value::Null,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DigestOutput {
    pub session_id: String,
    pub summary: String,
    #[serde(default)]
    pub recent_decisions: Vec<String>,
    #[serde(default)]
    pub active_hypotheses: Vec<String>,
    #[serde(default)]
    pub key_pointers: Vec<MemoryHandle>,
    #[serde(default)]
    pub failure_warnings: Vec<FailurePattern>,
    pub estimated_tokens: usize,
    pub generated_at: DateTime<Utc>,
}

impl DigestOutput {
    pub fn new(session_id: impl Into<String>, summary: impl Into<String>) -> Self {
        Self {
            session_id: session_id.into(),
            summary: summary.into(),
            recent_decisions: Vec::new(),
            active_hypotheses: Vec::new(),
            key_pointers: Vec::new(),
            failure_warnings: Vec::new(),
            estimated_tokens: 0,
            generated_at: Utc::now(),
        }
    }
}

/// Outbox entry for offline-first Change Data Capture (CDC) replication.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OutboxEntry {
    pub id: i64,
    pub record_id: String,
    pub entity_type: String,
    pub action: String,
    pub payload_json: String,
    pub created_at: DateTime<Utc>,
    pub retry_count: u32,
    pub last_attempt: Option<DateTime<Utc>>,
    pub status: String,
}

/// Health and status metric for offline-first CDC outbox.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OutboxStatus {
    pub pending_count: usize,
    pub synced_count: usize,
    pub failed_count: usize,
    pub last_synced_at: Option<DateTime<Utc>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_memory_tier_defaults_and_helpers() {
        let mem = MemoryRecord::new(MemoryType::Semantic, "Architecture constraint", Scope::Global);
        assert_eq!(mem.tier, MemoryTier::Peripheral);
        assert!(mem.is_peripheral());
        assert!(!mem.is_core());
        assert!(!mem.is_working());

        let core_mem = mem.with_tier(MemoryTier::Core);
        assert_eq!(core_mem.tier, MemoryTier::Core);
        assert!(core_mem.is_core());
        assert!(!core_mem.is_working());

        let working_handle = MemoryHandle {
            id: Uuid::new_v4(),
            title: "Task in progress".to_string(),
            summary: "Refactoring middleware".to_string(),
            memory_type: MemoryType::Episodic,
            scope: Scope::Global,
            tier: MemoryTier::Working,
            approved_by_human: false,
            relevance_score: Some(1.0),
        };
        assert!(working_handle.is_working());
        assert!(!working_handle.is_core());
        assert!(!working_handle.approved_by_human);
    }

    #[test]
    fn test_memory_tier_serde_and_parsing() {
        assert_eq!("core".parse::<MemoryTier>().unwrap(), MemoryTier::Core);
        assert_eq!("Core".parse::<MemoryTier>().unwrap(), MemoryTier::Core);
        assert_eq!("working".parse::<MemoryTier>().unwrap(), MemoryTier::Working);
        assert_eq!("peripheral".parse::<MemoryTier>().unwrap(), MemoryTier::Peripheral);

        let serialized = serde_json::to_string(&MemoryTier::Core).unwrap();
        assert_eq!(serialized, "\"core\"");
        let deserialized: MemoryTier = serde_json::from_str(&serialized).unwrap();
        assert_eq!(deserialized, MemoryTier::Core);
    }
}

