use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use strata_core::errors::StrataError;
use strata_core::events::Event;
use strata_core::state::{MemoryRecord, MemoryType, Scope};

// ============================================================================
// Distillation & Consolidation Data Structures
// ============================================================================

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EpisodicMemoryItem {
    pub summary: String,
    pub content: String,
    #[serde(default = "default_importance")]
    pub importance: f32,
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SemanticFact {
    #[serde(default)]
    pub id: Option<Uuid>,
    pub statement: String,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default = "default_importance")]
    pub importance: f32,
    #[serde(default = "default_confidence")]
    pub confidence: f32,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub scope: Option<String>,
    #[serde(default)]
    pub valid_from: Option<DateTime<Utc>>,
    #[serde(default = "default_status_active")]
    pub status: String,
    #[serde(default)]
    pub replaced_by: Option<Uuid>,
    #[serde(default = "default_version_1")]
    pub version: u32,
    #[serde(default)]
    pub metadata: serde_json::Value,
}

impl SemanticFact {
    pub fn new(statement: impl Into<String>) -> Self {
        Self {
            id: Some(Uuid::new_v4()),
            statement: statement.into(),
            summary: None,
            importance: 0.8,
            confidence: 1.0,
            tags: Vec::new(),
            scope: None,
            valid_from: Some(Utc::now()),
            status: "Active".to_string(),
            replaced_by: None,
            version: 1,
            metadata: serde_json::Value::Null,
        }
    }

    pub fn with_summary(mut self, summary: impl Into<String>) -> Self {
        self.summary = Some(summary.into());
        self
    }

    pub fn with_importance(mut self, importance: f32) -> Self {
        self.importance = importance.clamp(0.0, 1.0);
        self
    }

    pub fn with_tags(mut self, tags: Vec<String>) -> Self {
        self.tags = tags;
        self
    }

    pub fn with_version(mut self, version: u32) -> Self {
        self.version = version;
        self
    }

    pub fn deprecate(&mut self, replaced_by: Uuid) {
        self.status = "Deprecated".to_string();
        self.replaced_by = Some(replaced_by);
    }

    pub fn to_memory_record(&self, scope: Scope) -> MemoryRecord {
        let mut rec = MemoryRecord::new(MemoryType::Semantic, self.statement.clone(), scope)
            .with_importance(self.importance)
            .with_confidence(self.confidence)
            .with_tags(self.tags.clone());

        if let Some(ref id) = self.id {
            rec.id = *id;
        }

        if let Some(ref s) = self.summary {
            rec = rec.with_summary(s.clone());
        }

        let mut meta = if self.metadata.is_object() {
            self.metadata.clone()
        } else {
            serde_json::json!({})
        };

        meta["status"] = serde_json::json!(self.status);
        meta["version"] = serde_json::json!(self.version);
        if let Some(rb) = self.replaced_by {
            meta["replaced_by"] = serde_json::json!(rb.to_string());
        }

        rec.with_metadata(meta)
    }

    pub fn from_memory_record(record: &MemoryRecord) -> Self {
        let status = record
            .metadata
            .get("status")
            .and_then(|v| v.as_str())
            .unwrap_or("Active")
            .to_string();

        let version = record
            .metadata
            .get("version")
            .and_then(|v| v.as_u64())
            .unwrap_or(1) as u32;

        let replaced_by = record
            .metadata
            .get("replaced_by")
            .and_then(|v| v.as_str())
            .and_then(|s| Uuid::parse_str(s).ok());

        Self {
            id: Some(record.id),
            statement: record.content.clone(),
            summary: record.summary.clone(),
            importance: record.importance,
            confidence: record.confidence,
            tags: record.tags.clone(),
            scope: Some(record.scope.to_string()),
            valid_from: Some(record.created_at),
            status,
            replaced_by,
            version,
            metadata: record.metadata.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProceduralStep {
    pub step_number: u32,
    pub action: String,
    #[serde(default)]
    pub tool_name: Option<String>,
    #[serde(default)]
    pub expected_outcome: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProceduralSkill {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub trigger_conditions: Vec<String>,
    #[serde(default)]
    pub preconditions: Vec<String>,
    pub steps: Vec<ProceduralStep>,
    #[serde(default)]
    pub error_recovery: Option<String>,
    #[serde(default = "default_importance")]
    pub importance: f32,
    #[serde(default)]
    pub tags: Vec<String>,
}

impl ProceduralSkill {
    pub fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        steps: Vec<ProceduralStep>,
    ) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            trigger_conditions: Vec::new(),
            preconditions: Vec::new(),
            steps,
            error_recovery: None,
            importance: 0.75,
            tags: Vec::new(),
        }
    }

    pub fn to_memory_record(&self, scope: Scope) -> MemoryRecord {
        let mut content = format!("### Skill: {}\n{}\n\n", self.name, self.description);

        if !self.trigger_conditions.is_empty() {
            content.push_str("#### Trigger Conditions:\n");
            for tc in &self.trigger_conditions {
                content.push_str(&format!("- {tc}\n"));
            }
            content.push('\n');
        }

        if !self.preconditions.is_empty() {
            content.push_str("#### Preconditions:\n");
            for pc in &self.preconditions {
                content.push_str(&format!("- {pc}\n"));
            }
            content.push('\n');
        }

        content.push_str("#### Steps:\n");
        for step in &self.steps {
            let tool_str = step
                .tool_name
                .as_deref()
                .map(|t| format!(" [Tool: {t}]"))
                .unwrap_or_default();
            content.push_str(&format!(
                "{}. {}{}\n",
                step.step_number, step.action, tool_str
            ));
            if let Some(ref outcome) = step.expected_outcome {
                content.push_str(&format!("   -> Expected: {outcome}\n"));
            }
        }

        if let Some(ref recov) = self.error_recovery {
            content.push_str(&format!("\n#### Error Recovery:\n{recov}\n"));
        }

        let metadata = serde_json::json!({
            "skill_name": self.name,
            "trigger_conditions": self.trigger_conditions,
            "preconditions": self.preconditions,
            "steps": self.steps,
            "error_recovery": self.error_recovery,
        });

        MemoryRecord::new(MemoryType::Procedural, content, scope)
            .with_summary(format!("Procedural Skill: {}", self.name))
            .with_importance(self.importance)
            .with_tags(self.tags.clone())
            .with_metadata(metadata)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NegativePatternItem {
    pub signature: String,
    pub pattern_name: String,
    pub description: String,
    #[serde(default)]
    pub trigger_condition: String,
    pub mitigation: String,
    #[serde(default = "default_severity_medium")]
    pub severity: String,
    #[serde(default)]
    pub error_type: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct DistillationOutput {
    #[serde(default)]
    pub episodic_memories: Vec<EpisodicMemoryItem>,
    #[serde(default)]
    pub semantic_facts: Vec<SemanticFact>,
    #[serde(default)]
    pub procedural_skills: Vec<ProceduralSkill>,
    #[serde(default)]
    pub negative_patterns: Vec<NegativePatternItem>,
}

// ============================================================================
// JTMS (Justification-based Truth Maintenance System) Data Structures
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum JtmsClassification {
    Update,
    Duplicate,
    Refinement,
    Outlier,
}

impl std::fmt::Display for JtmsClassification {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            JtmsClassification::Update => write!(f, "update"),
            JtmsClassification::Duplicate => write!(f, "duplicate"),
            JtmsClassification::Refinement => write!(f, "refinement"),
            JtmsClassification::Outlier => write!(f, "outlier"),
        }
    }
}

impl std::str::FromStr for JtmsClassification {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_lowercase().as_str() {
            "update" | "contradiction" | "supersede" => Ok(JtmsClassification::Update),
            "duplicate" | "identical" | "redundant" => Ok(JtmsClassification::Duplicate),
            "refinement" | "addition" | "extension" => Ok(JtmsClassification::Refinement),
            "outlier" | "unrelated" | "independent" => Ok(JtmsClassification::Outlier),
            _ => Ok(JtmsClassification::Update),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JtmsArbitrationResult {
    pub classification: JtmsClassification,
    pub reason: String,
    #[serde(default = "default_confidence")]
    pub confidence: f32,
    #[serde(default)]
    pub replacement_id: Option<Uuid>,
    #[serde(default)]
    pub superseded_id: Option<Uuid>,
}

// ============================================================================
// Prompt Construction Functions
// ============================================================================

/// Generates the prompt requesting distillation JSON conforming to the 4 schemas:
/// `episodic_memories`, `semantic_facts`, `procedural_skills`, and `negative_patterns`.
pub fn build_distillation_prompt(events: &[Event]) -> String {
    let mut prompt = String::new();
    prompt.push_str("You are the cognitive memory consolidation engine for Strata.\n");
    prompt.push_str("Analyze the following stream of events and extract durable knowledge across 4 cognitive memory categories.\n\n");

    prompt.push_str("### OUTPUT FORMAT REQUIREMENTS:\n");
    prompt.push_str("You MUST return ONLY valid JSON matching this exact schema:\n");
    prompt.push_str(
        r#"{
  "episodic_memories": [
    {
      "summary": "Short headline summary of the session milestone or outcome",
      "content": "Detailed episodic narrative describing what happened and why",
      "importance": 0.7,
      "tags": ["session", "milestone"]
    }
  ],
  "semantic_facts": [
    {
      "statement": "Definitive architectural decision, invariant, or domain knowledge",
      "summary": "Short title (< 60 chars)",
      "importance": 0.9,
      "confidence": 1.0,
      "tags": ["architecture", "decision"]
    }
  ],
  "procedural_skills": [
    {
      "name": "Skill or workflow name",
      "description": "When and why to use this procedure",
      "trigger_conditions": ["Condition that triggers this workflow"],
      "preconditions": ["Prerequisites required before execution"],
      "steps": [
        {
          "step_number": 1,
          "action": "Action description",
          "tool_name": "tool_name_if_applicable",
          "expected_outcome": "Outcome to verify"
        }
      ],
      "error_recovery": "How to recover if a step fails",
      "importance": 0.8,
      "tags": ["workflow", "tool"]
    }
  ],
  "negative_patterns": [
    {
      "signature": "tool_or_action_failure_signature",
      "pattern_name": "Name of the failure anti-pattern",
      "description": "What caused the error or failure",
      "trigger_condition": "What parameter or action triggered it",
      "mitigation": "Actionable recommendation to prevent or resolve this failure",
      "severity": "high",
      "error_type": "ToolExecutionError"
    }
  ]
}"#,
    );
    prompt.push_str("\n\n### EVENT STREAM TO CONSOLIDATE:\n");

    for (i, event) in events.iter().enumerate() {
        let ts = event.timestamp.to_rfc3339();
        let payload_json = serde_json::to_string(&event.payload).unwrap_or_default();
        prompt.push_str(&format!(
            "[{}] Seq: {:?}, Type: {}, Time: {}\nPayload: {}\n\n",
            i + 1,
            event.sequence,
            event.payload.event_type(),
            ts,
            payload_json
        ));
    }

    prompt.push_str("Generate the consolidated JSON now:\n");
    prompt
}

/// Generates the JTMS arbitration prompt comparing an existing semantic fact with a newly candidate fact.
/// Classifies relation into: `update` (contradiction/superseded), `duplicate` (same knowledge),
/// `refinement` (adds new detail), or `outlier` (independent fact).
pub fn build_jtms_arbitration_prompt(old_fact: &SemanticFact, new_fact: &SemanticFact) -> String {
    let mut prompt = String::new();
    prompt.push_str(
        "You are the Justification-based Truth Maintenance System (JTMS) arbitrator for Strata.\n",
    );
    prompt.push_str("Compare the existing semantic fact against the newly observed candidate fact to resolve belief revisions.\n\n");

    prompt.push_str("### EXISTING FACT:\n");
    prompt.push_str(&format!("- Statement: {}\n", old_fact.statement));
    if let Some(ref s) = old_fact.summary {
        prompt.push_str(&format!("- Summary: {}\n", s));
    }
    prompt.push_str(&format!("- Version: {}\n", old_fact.version));
    prompt.push_str(&format!("- Status: {}\n", old_fact.status));
    prompt.push_str(&format!("- Tags: {}\n\n", old_fact.tags.join(", ")));

    prompt.push_str("### NEW CANDIDATE FACT:\n");
    prompt.push_str(&format!("- Statement: {}\n", new_fact.statement));
    if let Some(ref s) = new_fact.summary {
        prompt.push_str(&format!("- Summary: {}\n", s));
    }
    prompt.push_str(&format!("- Tags: {}\n\n", new_fact.tags.join(", ")));

    prompt.push_str("### CLASSIFICATION CATEGORIES:\n");
    prompt.push_str("1. `update`: The new fact directly supersedes, replaces, or contradicts the existing fact (e.g. migration, technology change).\n");
    prompt.push_str(
        "2. `duplicate`: The new fact conveys the same information without substantial changes.\n",
    );
    prompt.push_str("3. `refinement`: The new fact adds detail, context, or precision while keeping the core fact valid.\n");
    prompt.push_str("4. `outlier`: The facts discuss different aspects and both remain independently active.\n\n");

    prompt.push_str("### OUTPUT FORMAT:\n");
    prompt.push_str("Respond with JSON ONLY in this format:\n");
    prompt.push_str(
        r#"{
  "classification": "update",
  "reason": "Brief rationale for this decision",
  "confidence": 0.95
}"#,
    );
    prompt.push('\n');

    prompt
}

/// Helper to parse a distillation JSON string into a structured `DistillationOutput`.
pub fn parse_distillation_output(raw_json: &str) -> Result<DistillationOutput, StrataError> {
    let cleaned = clean_json_string(raw_json);
    serde_json::from_str::<DistillationOutput>(&cleaned).map_err(|e| {
        StrataError::Reasoning(format!(
            "Failed to parse distillation JSON: {e} (raw: {cleaned})"
        ))
    })
}

/// Helper to parse a JTMS arbitration JSON string into `JtmsArbitrationResult`.
pub fn parse_jtms_arbitration(raw_json: &str) -> Result<JtmsArbitrationResult, StrataError> {
    let cleaned = clean_json_string(raw_json);
    serde_json::from_str::<JtmsArbitrationResult>(&cleaned).map_err(|e| {
        StrataError::Reasoning(format!("Failed to parse JTMS JSON: {e} (raw: {cleaned})"))
    })
}

fn clean_json_string(s: &str) -> String {
    let trimmed = s.trim();
    if trimmed.starts_with("```json") {
        if let Some(end) = trimmed.rfind("```") {
            if end > 7 {
                return trimmed[7..end].trim().to_string();
            }
        }
    } else if trimmed.starts_with("```") {
        if let Some(end) = trimmed.rfind("```") {
            if end > 3 {
                return trimmed[3..end].trim().to_string();
            }
        }
    }
    trimmed.to_string()
}

// Default helper functions for serde
fn default_importance() -> f32 {
    0.7
}
fn default_confidence() -> f32 {
    1.0
}
fn default_status_active() -> String {
    "Active".to_string()
}
fn default_version_1() -> u32 {
    1
}
fn default_severity_medium() -> String {
    "medium".to_string()
}
