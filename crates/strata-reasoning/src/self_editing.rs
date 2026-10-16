use std::sync::Arc;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use strata_core::errors::StrataError;
use crate::engine::{ChatMessage, PromptContext, ReasoningEngine};

/// Text gradient feedback capturing criticism, optional developer guidance,
/// execution traces, and scalar evaluation scores.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TextGradientFeedback {
    pub criticism: String,
    pub developer_correction: Option<String>,
    pub execution_trace: Option<String>,
    pub score: Option<f32>,
}

impl TextGradientFeedback {
    pub fn new(criticism: impl Into<String>) -> Self {
        Self {
            criticism: criticism.into(),
            developer_correction: None,
            execution_trace: None,
            score: None,
        }
    }

    pub fn with_correction(mut self, correction: impl Into<String>) -> Self {
        self.developer_correction = Some(correction.into());
        self
    }

    pub fn with_trace(mut self, trace: impl Into<String>) -> Self {
        self.execution_trace = Some(trace.into());
        self
    }

    pub fn with_score(mut self, score: f32) -> Self {
        self.score = Some(score);
        self
    }
}

/// Immutable record of a procedural rule version across optimization iterations.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProceduralRuleVersion {
    pub version: u32,
    pub content: String,
    pub rationale: String,
    pub feedback_applied: Option<TextGradientFeedback>,
    pub timestamp: DateTime<Utc>,
}

/// A versioned procedural rule with rollback capabilities and text gradient history.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProceduralRule {
    pub id: String,
    pub name: String,
    pub category: String,
    pub current_version: u32,
    pub current_content: String,
    pub versions: Vec<ProceduralRuleVersion>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl ProceduralRule {
    pub fn new(name: impl Into<String>, content: impl Into<String>) -> Self {
        let id = Uuid::new_v4().to_string();
        let name_str = name.into();
        let content_str = content.into();
        let now = Utc::now();
        let initial_version = ProceduralRuleVersion {
            version: 1,
            content: content_str.clone(),
            rationale: "Initial baseline version".to_string(),
            feedback_applied: None,
            timestamp: now,
        };

        Self {
            id,
            name: name_str,
            category: "general".to_string(),
            current_version: 1,
            current_content: content_str,
            versions: vec![initial_version],
            created_at: now,
            updated_at: now,
        }
    }

    pub fn with_id(mut self, id: impl Into<String>) -> Self {
        self.id = id.into();
        self
    }

    pub fn with_category(mut self, cat: impl Into<String>) -> Self {
        self.category = cat.into();
        self
    }

    pub fn apply_version(
        &mut self,
        new_content: impl Into<String>,
        rationale: impl Into<String>,
        feedback: Option<TextGradientFeedback>,
    ) -> ProceduralRuleVersion {
        let next_version = self.current_version + 1;
        let content_str = new_content.into();
        let now = Utc::now();
        let ver = ProceduralRuleVersion {
            version: next_version,
            content: content_str.clone(),
            rationale: rationale.into(),
            feedback_applied: feedback,
            timestamp: now,
        };
        self.current_version = next_version;
        self.current_content = content_str;
        self.updated_at = now;
        self.versions.push(ver.clone());
        ver
    }

    pub fn rollback_to(&mut self, version: u32) -> bool {
        if let Some(target) = self.versions.iter().find(|v| v.version == version) {
            self.current_version = target.version;
            self.current_content = target.content.clone();
            self.updated_at = Utc::now();
            true
        } else {
            false
        }
    }

    pub fn get_version(&self, version: u32) -> Option<&ProceduralRuleVersion> {
        self.versions.iter().find(|v| v.version == version)
    }
}

/// Hyperparameters for procedural rule optimization via text gradients.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProceduralOptimizerConfig {
    pub learning_rate_guidance: String,
    pub max_versions: usize,
    pub temperature: f32,
}

impl Default for ProceduralOptimizerConfig {
    fn default() -> Self {
        Self {
            learning_rate_guidance: "conservative incremental edits preserving core semantics and structure".to_string(),
            max_versions: 50,
            temperature: 0.2,
        }
    }
}

impl ProceduralOptimizerConfig {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_learning_rate_guidance(mut self, guidance: impl Into<String>) -> Self {
        self.learning_rate_guidance = guidance.into();
        self
    }

    pub fn with_max_versions(mut self, max: usize) -> Self {
        self.max_versions = max;
        self
    }

    pub fn with_temperature(mut self, temp: f32) -> Self {
        self.temperature = temp;
        self
    }
}

fn parse_optimization_json(content: &str) -> Option<(String, String)> {
    let clean = if let Some(start) = content.find("```json") {
        let rest = &content[start + 7..];
        if let Some(end) = rest.find("```") {
            &rest[..end]
        } else {
            rest
        }
    } else if let Some(start) = content.find("```") {
        let rest = &content[start + 3..];
        if let Some(end) = rest.find("```") {
            &rest[..end]
        } else {
            rest
        }
    } else if let (Some(first_brace), Some(last_brace)) = (content.find('{'), content.rfind('}')) {
        if first_brace <= last_brace {
            &content[first_brace..=last_brace]
        } else {
            content
        }
    } else {
        content
    };

    #[derive(Deserialize)]
    struct LlmOutput {
        revised_content: Option<String>,
        content: Option<String>,
        revised_rule: Option<String>,
        rationale: Option<String>,
        reasoning: Option<String>,
    }

    if let Ok(parsed) = serde_json::from_str::<LlmOutput>(clean.trim()) {
        let content = parsed
            .revised_content
            .or(parsed.content)
            .or(parsed.revised_rule)?;
        let rationale = parsed
            .rationale
            .or(parsed.reasoning)
            .unwrap_or_else(|| "Optimized procedural rule via reasoning engine".to_string());
        Some((content, rationale))
    } else {
        None
    }
}

/// Engine for synthesizing rule revisions via natural language text gradients.
pub struct ProceduralSelfEditor {
    engine: Option<Arc<dyn ReasoningEngine>>,
    config: ProceduralOptimizerConfig,
}

impl ProceduralSelfEditor {
    pub fn new() -> Self {
        Self {
            engine: None,
            config: ProceduralOptimizerConfig::default(),
        }
    }

    pub fn with_engine(mut self, engine: Arc<dyn ReasoningEngine>) -> Self {
        self.engine = Some(engine);
        self
    }

    pub fn with_config(mut self, config: ProceduralOptimizerConfig) -> Self {
        self.config = config;
        self
    }

    pub fn build_optimization_prompt(
        rule: &ProceduralRule,
        feedback: &TextGradientFeedback,
    ) -> PromptContext {
        let system_prompt = r#"You are an expert procedural instruction optimizer and text gradient prompt engineer.
Your task is to refine a procedural rule based on natural language text gradient feedback (criticisms, corrections, execution traces, and performance scores).

Optimizer Guidelines:
1. Preserve all existing valid constraints, steps, and core semantics of the rule.
2. Directly incorporate the developer correction and address the criticism precisely.
3. Make conservative, targeted improvements following the text gradient.

You MUST respond strictly with a valid JSON object matching this schema:
{
  "revised_content": "<complete updated text of the procedural rule>",
  "rationale": "<concise explanation of changes made to address the text gradient feedback>"
}"#;

        let mut user_msg = format!(
            "### Target Procedural Rule\nID: {}\nName: {}\nCategory: {}\nCurrent Version: {}\n\n--- Current Content ---\n{}\n-----------------------\n\n### Text Gradient Feedback\nCriticism: {}\n",
            rule.id, rule.name, rule.category, rule.current_version, rule.current_content, feedback.criticism
        );

        if let Some(corr) = &feedback.developer_correction {
            user_msg.push_str(&format!("Developer Correction: {}\n", corr));
        }
        if let Some(trace) = &feedback.execution_trace {
            user_msg.push_str(&format!("Execution Trace: {}\n", trace));
        }
        if let Some(score) = feedback.score {
            user_msg.push_str(&format!("Evaluation Score: {:.2}\n", score));
        }

        user_msg.push_str("\nPlease refine the procedural rule and return the updated content with a rationale in JSON.");

        PromptContext::new()
            .with_system(system_prompt)
            .with_message(ChatMessage::user(user_msg))
            .with_temperature(0.2)
    }

    fn deterministic_synthesis(
        &self,
        rule: &ProceduralRule,
        feedback: &TextGradientFeedback,
    ) -> (String, String) {
        let addition = if let Some(corr) = &feedback.developer_correction {
            let trimmed = corr.trim();
            if trimmed.starts_with('-') || trimmed.starts_with('*') || trimmed.starts_with("1.") {
                trimmed.to_string()
            } else {
                format!("- {}", trimmed)
            }
        } else {
            format!("- Address: {}", feedback.criticism.trim())
        };

        let revised_content = if rule.current_content.trim().is_empty() {
            addition
        } else {
            format!("{}\n{}", rule.current_content.trim_end(), addition)
        };

        let rationale = if let Some(corr) = &feedback.developer_correction {
            format!(
                "Refined rule to address critique '{}' by applying developer correction: {}",
                feedback.criticism, corr
            )
        } else {
            format!(
                "Refined rule to address critique: {}",
                feedback.criticism
            )
        };

        (revised_content, rationale)
    }

    pub async fn optimize_rule(
        &self,
        rule: &mut ProceduralRule,
        feedback: TextGradientFeedback,
    ) -> Result<ProceduralRuleVersion, StrataError> {
        let prompt_ctx = Self::build_optimization_prompt(rule, &feedback)
            .with_temperature(self.config.temperature);

        let (revised_content, rationale) = match &self.engine {
            Some(engine) => match engine.complete(&prompt_ctx).await {
                Ok(output) => {
                    if let Some(text) = output.content {
                        parse_optimization_json(&text)
                            .unwrap_or_else(|| self.deterministic_synthesis(rule, &feedback))
                    } else {
                        self.deterministic_synthesis(rule, &feedback)
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        "Reasoning engine error during rule optimization ({:?}), falling back to deterministic synthesis",
                        e
                    );
                    self.deterministic_synthesis(rule, &feedback)
                }
            },
            None => self.deterministic_synthesis(rule, &feedback),
        };

        let ver = rule.apply_version(revised_content, rationale, Some(feedback));

        if self.config.max_versions > 0 && rule.versions.len() > self.config.max_versions {
            let excess = rule.versions.len() - self.config.max_versions;
            for _ in 0..excess {
                if rule.versions.len() > 1 {
                    rule.versions.remove(1);
                }
            }
        }

        Ok(ver)
    }

    pub async fn multi_round_optimize(
        &self,
        rule: &mut ProceduralRule,
        rounds: Vec<TextGradientFeedback>,
    ) -> Result<Vec<ProceduralRuleVersion>, StrataError> {
        let mut results = Vec::new();
        for fb in rounds {
            results.push(self.optimize_rule(rule, fb).await?);
        }
        Ok(results)
    }

    pub async fn refine_prompt_text(
        &self,
        prompt: &str,
        feedback: &[TextGradientFeedback],
    ) -> Result<String, StrataError> {
        let mut rule = ProceduralRule::new("prompt_refinement", prompt);
        for fb in feedback {
            self.optimize_rule(&mut rule, fb.clone()).await?;
        }
        Ok(rule.current_content)
    }
}

impl Default for ProceduralSelfEditor {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock::MockReasoningEngine;
    use serde_json::json;

    #[test]
    fn test_baseline_prompt_construction_for_text_gradient() {
        let rule = ProceduralRule::new(
            "strict_tdd_protocol",
            "# TDD Protocol\nAlways write unit test before production code.",
        )
        .with_id("rule-tdd-1")
        .with_category("engineering");

        let feedback = TextGradientFeedback::new("Timing violation: production code written before test failed")
            .with_correction("Enforce Red phase verification before any implementation")
            .with_trace("Execution trace: src/lib.rs modified prior to test creation")
            .with_score(0.2);

        let ctx = ProceduralSelfEditor::build_optimization_prompt(&rule, &feedback);

        assert!(ctx.system_prompt.is_some(), "System prompt should be present");
        let sys = ctx.system_prompt.unwrap();
        assert!(sys.contains("text gradient") || sys.contains("optimizer"));

        assert_eq!(ctx.messages.len(), 1);
        let user_msg = &ctx.messages[0].content;
        assert!(user_msg.contains("strict_tdd_protocol"));
        assert!(user_msg.contains("Timing violation: production code written before test failed"));
        assert!(user_msg.contains("Enforce Red phase verification before any implementation"));
        assert!(user_msg.contains("Execution trace: src/lib.rs modified prior to test creation"));
    }

    #[tokio::test]
    async fn test_progressive_refinement_three_consecutive_rounds() {
        let editor = ProceduralSelfEditor::new();
        let mut rule = ProceduralRule::new(
            "agent_coding_rules",
            "# Agent Coding Protocol\n1. Follow user instructions.",
        );

        assert_eq!(rule.current_version, 1);
        assert_eq!(rule.versions.len(), 1);

        // Round 1: Strict TDD timing
        let round1 = TextGradientFeedback::new("Agent wrote implementation before verifying test failure")
            .with_correction("Strict TDD timing: Write failing test first and execute it");
        let v2 = editor.optimize_rule(&mut rule, round1).await.unwrap();

        assert_eq!(rule.current_version, 2);
        assert_eq!(v2.version, 2);
        assert!(rule.current_content.contains("Strict TDD timing"));
        assert!(v2.rationale.contains("Strict TDD timing") || v2.rationale.contains("Agent wrote implementation"));

        // Round 2: Check failure reason in Red phase
        let round2 = TextGradientFeedback::new("Test failed due to syntax error instead of expected assertion")
            .with_correction("Verify Red phase failure reason matches expected assertion failure");
        let v3 = editor.optimize_rule(&mut rule, round2).await.unwrap();

        assert_eq!(rule.current_version, 3);
        assert_eq!(v3.version, 3);
        assert!(rule.current_content.contains("Verify Red phase failure reason"));
        assert!(rule.current_content.contains("Strict TDD timing"));

        // Round 3: Mock all network/disk I/O
        let round3 = TextGradientFeedback::new("Test executed actual network I/O causing non-deterministic flakiness")
            .with_correction("Mock all network and disk I/O in unit test suites");
        let v4 = editor.optimize_rule(&mut rule, round3).await.unwrap();

        assert_eq!(rule.current_version, 4);
        assert_eq!(v4.version, 4);
        assert!(rule.current_content.contains("Mock all network and disk I/O"));
        assert!(rule.current_content.contains("Verify Red phase failure reason"));
        assert!(rule.current_content.contains("Strict TDD timing"));

        assert_eq!(rule.versions.len(), 4);
    }

    #[tokio::test]
    async fn test_llm_reasoning_engine_mock_integration() {
        let mock = Arc::new(MockReasoningEngine::new());

        // Push JSON responses for optimization
        mock.push_json(&json!({
            "revised_content": "# TDD Protocol\n1. Write failing test.\n2. Verify failure reason.\n3. Implement minimal code.",
            "rationale": "Incorporated test failure verification step into TDD protocol"
        })).await;

        let editor = ProceduralSelfEditor::new().with_engine(mock.clone());
        let mut rule = ProceduralRule::new(
            "tdd_rule",
            "# TDD Protocol\n1. Write test.\n2. Implement code.",
        );

        let feedback = TextGradientFeedback::new("Skipped Red phase failure inspection")
            .with_correction("Add failure inspection step");

        let v2 = editor.optimize_rule(&mut rule, feedback).await.unwrap();

        assert_eq!(rule.current_version, 2);
        assert_eq!(v2.version, 2);
        assert!(rule.current_content.contains("Verify failure reason"));
        assert_eq!(v2.rationale, "Incorporated test failure verification step into TDD protocol");

        mock.assert_called_times(1).await;
    }

    #[test]
    fn test_rollback_functionality() {
        let mut rule = ProceduralRule::new("deploy_rule", "v1 content");
        assert_eq!(rule.current_version, 1);

        rule.apply_version("v2 content with canary", "added canary deployment", None);
        assert_eq!(rule.current_version, 2);
        assert_eq!(rule.current_content, "v2 content with canary");

        rule.apply_version("v3 content broken", "experimental change", None);
        assert_eq!(rule.current_version, 3);
        assert_eq!(rule.current_content, "v3 content broken");

        // Rollback to v2
        let success = rule.rollback_to(2);
        assert!(success);
        assert_eq!(rule.current_version, 2);
        assert_eq!(rule.current_content, "v2 content with canary");

        // Rollback to non-existent version 99
        let invalid = rule.rollback_to(99);
        assert!(!invalid);
        assert_eq!(rule.current_version, 2);
    }

    #[tokio::test]
    async fn test_refine_prompt_text_helper() {
        let editor = ProceduralSelfEditor::new();
        let prompt = "System: Analyze code.";
        let feedback = vec![
            TextGradientFeedback::new("Need security emphasis")
                .with_correction("Check for memory leaks and race conditions"),
        ];

        let refined = editor.refine_prompt_text(prompt, &feedback).await.unwrap();
        assert!(refined.contains("Check for memory leaks and race conditions"));
    }
}
