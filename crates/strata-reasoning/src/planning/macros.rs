use anyhow::{bail, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::Instant;
use strata_core::schemas::{ParameterDef, ProceduralSkill, ProceduralStep};
use uuid::Uuid;

use super::dag::GoalDag;
use super::types::{GoalNode, GoalNodeKind, GoalStatus};

/// Detailed record of a single procedural macro substitution in the Goal DAG.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MacroSubstitution {
    pub original_node_id: String,
    pub original_title: String,
    pub skill_name: String,
    pub skill_id: uuid::Uuid,
    pub generated_step_ids: Vec<String>,
    pub steps_count: usize,
    pub success_rate: f32,
}

/// Compilation report summarizing macro substitutions executed across a Goal DAG.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct MacroCompilationReport {
    pub substitutions: Vec<MacroSubstitution>,
    pub total_steps_compiled: usize,
    pub replanning_bypassed: bool,
}

impl MacroCompilationReport {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn has_substitutions(&self) -> bool {
        !self.substitutions.is_empty()
    }
}

/// A parameter required by a compiled procedural macro.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MacroParameter {
    pub name: String,
    pub description: String,
    pub default_value: Option<serde_json::Value>,
    pub required: bool,
}

impl MacroParameter {
    pub fn new(name: impl Into<String>, description: impl Into<String>, required: bool) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            default_value: None,
            required,
        }
    }

    pub fn with_default(mut self, default_val: serde_json::Value) -> Self {
        self.default_value = Some(default_val);
        self
    }

    pub fn to_parameter_def(&self) -> ParameterDef {
        ParameterDef {
            name: self.name.clone(),
            param_type: "string".to_string(),
            description: self.description.clone(),

            example: self.default_value.as_ref().map(|v| match v {
                serde_json::Value::String(s) => s.clone(),
                other => other.to_string(),
            }),
        }
    }
}

/// A single step in a compiled macro with argument templates.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MacroStep {
    pub step_index: u32,
    pub tool_name: String,
    pub action: String,
    pub input_template: serde_json::Value,
    #[serde(default)]
    pub expected_output: Option<String>,
}

impl MacroStep {
    pub fn new(
        step_index: u32,
        tool_name: impl Into<String>,
        action: impl Into<String>,
        input_template: serde_json::Value,
    ) -> Self {
        Self {
            step_index,
            tool_name: tool_name.into(),
            action: action.into(),
            input_template,
            expected_output: None,
        }
    }

    pub fn with_expected_output(mut self, expected: impl Into<String>) -> Self {
        self.expected_output = Some(expected.into());
        self
    }
}

/// A compiled procedural macro synthesized from recurrent successful execution patterns.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CompiledMacro {
    pub id: Uuid,
    pub name: String,
    pub description: String,
    pub tool_sequence: Vec<String>,
    pub parameters: Vec<MacroParameter>,
    pub steps: Vec<MacroStep>,
    pub preconditions: Vec<String>,
    pub postconditions: Vec<String>,
    pub support_count: usize,
    pub success_rate: f32,
    pub estimated_tokens_saved: usize,
    pub created_at: DateTime<Utc>,
}

impl CompiledMacro {
    pub fn new(name: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            description: description.into(),
            tool_sequence: Vec::new(),
            parameters: Vec::new(),
            steps: Vec::new(),
            preconditions: Vec::new(),
            postconditions: Vec::new(),
            support_count: 1,
            success_rate: 1.0,
            estimated_tokens_saved: 0,
            created_at: Utc::now(),
        }
    }

    pub fn with_steps(mut self, steps: Vec<MacroStep>) -> Self {
        self.tool_sequence = steps.iter().map(|s| s.tool_name.clone()).collect();
        self.estimated_tokens_saved = steps.len() * 450;
        self.steps = steps;
        self
    }

    pub fn with_parameters(mut self, parameters: Vec<MacroParameter>) -> Self {
        self.parameters = parameters;
        self
    }

    pub fn with_preconditions(mut self, pre: Vec<String>) -> Self {
        self.preconditions = pre;
        self
    }

    pub fn with_postconditions(mut self, post: Vec<String>) -> Self {
        self.postconditions = post;
        self
    }

    pub fn to_procedural_skill(&self) -> ProceduralSkill {
        let mut skill = ProceduralSkill::new(self.name.clone(), self.description.clone())
            .with_preconditions(self.preconditions.clone())
            .with_postconditions(self.postconditions.clone())
            .with_parameters(
                self.parameters
                    .iter()
                    .map(|p| p.to_parameter_def())
                    .collect(),
            );
        skill.id = self.id;
        skill.success_rate = self.success_rate;
        skill.importance = 0.90;
        skill.tags = vec!["compiled_macro".to_string(), "autonomous_mined".to_string()];
        skill.tags.extend(self.tool_sequence.clone());
        skill.steps = self
            .steps
            .iter()
            .map(|s| {
                let mut step = ProceduralStep::new(
                    s.step_index,
                    &s.tool_name,
                    &s.action,
                    s.input_template.clone(),
                );
                if let Some(ref exp) = s.expected_output {
                    step = step.with_expected_result(exp);
                }
                step
            })
            .collect();
        skill
    }

    /// Reconstructs a CompiledMacro from a persisted ProceduralSkill.
    pub fn from_procedural_skill(skill: &ProceduralSkill) -> Self {
        let tool_sequence: Vec<String> = skill.steps.iter().map(|s| s.tool.clone()).collect();
        let parameters: Vec<MacroParameter> = skill
            .parameters
            .iter()
            .map(|p| MacroParameter {
                name: p.name.clone(),
                description: p.description.clone(),
                default_value: p
                    .example
                    .as_ref()
                    .and_then(|ex| serde_json::from_str(ex).ok()),
                required: true,
            })
            .collect();

        let steps: Vec<MacroStep> = skill
            .steps
            .iter()
            .map(|s| MacroStep {
                step_index: s.order,
                tool_name: s.tool.clone(),
                action: s.action.clone(),
                input_template: s.arguments.clone(),
                expected_output: s.expected_result.clone(),
            })
            .collect();

        let est = steps.len() * 450;
        Self {
            id: skill.id,
            name: skill.name.clone(),
            description: skill.description.clone(),
            tool_sequence,
            parameters,
            steps,
            preconditions: skill.preconditions.clone(),
            postconditions: skill.postconditions.clone(),
            support_count: 1,
            estimated_tokens_saved: est,
            success_rate: skill.success_rate,
            created_at: skill.created_at,
        }
    }
}

/// Result of a single step within a 1-step macro execution.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StepExecutionResult {
    pub step_index: u32,
    pub tool_name: String,
    pub success: bool,
    pub output: serde_json::Value,
    pub error: Option<String>,
    pub duration_ms: u64,
}

/// Unified execution report returned by the 1-step macro engine.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MacroExecutionReport {
    pub macro_id: Uuid,
    pub macro_name: String,
    pub success: bool,
    pub steps_completed: usize,
    pub total_steps: usize,
    pub step_results: Vec<StepExecutionResult>,
    pub total_duration_ms: u64,
    pub tokens_saved: usize,
    pub final_output: serde_json::Value,
    pub error: Option<String>,
}

/// Invoker trait abstraction for tool execution during 1-step macro execution.
#[async_trait::async_trait]
pub trait ToolInvoker: Send + Sync {
    async fn invoke(&self, tool: &str, input: &serde_json::Value) -> Result<serde_json::Value>;
}

/// Mock/Function-based tool invoker for tests and lightweight runtime execution.
pub struct CallbackToolInvoker<F> {
    callback: F,
}

impl<F> CallbackToolInvoker<F> {
    pub fn new(callback: F) -> Self {
        Self { callback }
    }
}

#[async_trait::async_trait]
impl<F> ToolInvoker for CallbackToolInvoker<F>
where
    F: Fn(&str, &serde_json::Value) -> Result<serde_json::Value> + Send + Sync,
{
    async fn invoke(&self, tool: &str, input: &serde_json::Value) -> Result<serde_json::Value> {
        (self.callback)(tool, input)
    }
}

/// 1-Step Macro Execution Engine (Phase 4.3).
///
/// Executes all steps of a pre-compiled macro sequentially in native code,
/// interpolating parameters without LLM round-trips.
#[derive(Debug, Clone, Default)]
pub struct MacroExecutionEngine;

impl MacroExecutionEngine {
    pub fn new() -> Self {
        Self
    }

    /// Interpolates variable slots `{{var_name}}` in JSON templates with provided parameters.
    pub fn interpolate_template(
        template: &serde_json::Value,
        params: &HashMap<String, serde_json::Value>,
    ) -> serde_json::Value {
        match template {
            serde_json::Value::String(s) => {
                let mut interpolated = s.clone();
                for (k, v) in params {
                    let placeholder = format!("{{{{{k}}}}}");
                    let replacement = match v {
                        serde_json::Value::String(vs) => vs.clone(),
                        other => other.to_string(),
                    };
                    interpolated = interpolated.replace(&placeholder, &replacement);
                }
                serde_json::Value::String(interpolated)
            }
            serde_json::Value::Object(map) => {
                let mut new_map = serde_json::Map::new();
                for (k, v) in map {
                    new_map.insert(k.clone(), Self::interpolate_template(v, params));
                }
                serde_json::Value::Object(new_map)
            }
            serde_json::Value::Array(arr) => {
                let new_arr = arr
                    .iter()
                    .map(|item| Self::interpolate_template(item, params))
                    .collect();
                serde_json::Value::Array(new_arr)
            }
            other => other.clone(),
        }
    }

    /// Executes all steps of a compiled macro in a single unified step.
    pub async fn execute_macro_1step(
        &self,
        macro_def: &CompiledMacro,
        params: &HashMap<String, serde_json::Value>,
        invoker: &dyn ToolInvoker,
    ) -> Result<MacroExecutionReport> {
        let start = Instant::now();
        let mut step_results = Vec::new();
        let mut final_output = serde_json::json!({});
        let mut success = true;
        let mut failure_err = None;

        // 1. Validate required parameters
        for param in &macro_def.parameters {
            if param.required && !params.contains_key(&param.name) && param.default_value.is_none()
            {
                bail!(
                    "Missing required parameter '{}' for macro '{}'",
                    param.name,
                    macro_def.name
                );
            }
        }

        // Merge defaults with provided params
        let mut effective_params = params.clone();
        for param in &macro_def.parameters {
            if !effective_params.contains_key(&param.name) {
                if let Some(ref def) = param.default_value {
                    effective_params.insert(param.name.clone(), def.clone());
                }
            }
        }

        // 2. Execute each step sequentially
        for step in &macro_def.steps {
            let step_start = Instant::now();
            let resolved_input =
                Self::interpolate_template(&step.input_template, &effective_params);

            match invoker.invoke(&step.tool_name, &resolved_input).await {
                Ok(output) => {
                    let duration = step_start.elapsed().as_millis() as u64;
                    final_output = output.clone();
                    step_results.push(StepExecutionResult {
                        step_index: step.step_index,
                        tool_name: step.tool_name.clone(),
                        success: true,
                        output,
                        error: None,
                        duration_ms: duration,
                    });
                }
                Err(err) => {
                    let duration = step_start.elapsed().as_millis() as u64;
                    let err_str = err.to_string();
                    step_results.push(StepExecutionResult {
                        step_index: step.step_index,
                        tool_name: step.tool_name.clone(),
                        success: false,
                        output: serde_json::json!(null),
                        error: Some(err_str.clone()),
                        duration_ms: duration,
                    });
                    success = false;
                    failure_err = Some(format!(
                        "Step {} ({}) failed: {}",
                        step.step_index, step.tool_name, err_str
                    ));
                    break;
                }
            }
        }

        let total_duration = start.elapsed().as_millis() as u64;
        let steps_completed = step_results.iter().filter(|s| s.success).count();

        Ok(MacroExecutionReport {
            macro_id: macro_def.id,
            macro_name: macro_def.name.clone(),
            success,
            steps_completed,
            total_steps: macro_def.steps.len(),
            step_results,
            total_duration_ms: total_duration,
            tokens_saved: macro_def.estimated_tokens_saved,
            final_output,
            error: failure_err,
        })
    }
}

/// Deterministic compiler that matches Goal DAG subgoals against learned procedural skills,
/// substituting high-level goals with pre-verified action macros without LLM re-planning.
#[derive(Debug, Clone)]
pub struct ProceduralMacroCompiler {
    skills: Vec<ProceduralSkill>,
    min_success_rate: f32,
    auto_inject_verification: bool,
}

impl Default for ProceduralMacroCompiler {
    fn default() -> Self {
        Self::new()
    }
}

impl ProceduralMacroCompiler {
    pub fn new() -> Self {
        Self {
            skills: Vec::new(),
            min_success_rate: 0.70,
            auto_inject_verification: true,
        }
    }

    pub fn with_skills(mut self, skills: Vec<ProceduralSkill>) -> Self {
        self.skills = skills;
        self
    }

    pub fn add_skill(&mut self, skill: ProceduralSkill) {
        self.skills.push(skill);
    }

    pub fn add_macro(&mut self, macro_def: &CompiledMacro) {
        self.skills.push(macro_def.to_procedural_skill());
    }

    pub fn with_min_success_rate(mut self, min_rate: f32) -> Self {
        self.min_success_rate = min_rate.clamp(0.0, 1.0);
        self
    }

    pub fn with_auto_inject_verification(mut self, inject: bool) -> Self {
        self.auto_inject_verification = inject;
        self
    }

    pub fn skills(&self) -> &[ProceduralSkill] {
        &self.skills
    }

    /// Matches a goal node against the procedural skill registry using precondition and invariant matching.
    pub fn find_best_macro_for_goal(&self, goal: &GoalNode) -> Option<&ProceduralSkill> {
        // Skip root nodes or verification gates unless explicitly targeted
        if matches!(
            goal.kind,
            GoalNodeKind::Root | GoalNodeKind::Verification | GoalNodeKind::Rollback
        ) {
            return None;
        }

        let goal_title_lower = goal.title.to_lowercase();
        let goal_desc_lower = goal.description.to_lowercase();
        let goal_action = goal.action.as_deref().unwrap_or("").to_lowercase();

        let mut best_match: Option<(&ProceduralSkill, f32)> = None;

        for skill in &self.skills {
            // Must meet minimum historical success rate
            if skill.success_rate < self.min_success_rate || skill.steps.is_empty() {
                continue;
            }

            let mut match_score = 0.0f32;
            let skill_name_clean = skill.name.replace('_', " ").to_lowercase();
            let skill_desc_clean = skill.description.to_lowercase();

            // 1. Precondition matching: check if goal title or metadata matches declared task domain
            for pre in &skill.preconditions {
                let pre_lower = pre.to_lowercase();
                if pre_lower.starts_with("task domain:") {
                    let domain = pre_lower.trim_start_matches("task domain:").trim();
                    if goal_title_lower.contains(domain) || domain.contains(&goal_title_lower) {
                        match_score += 0.8;
                    }
                } else if goal_title_lower.contains(&pre_lower)
                    || goal_desc_lower.contains(&pre_lower)
                {
                    match_score += 0.5;
                }
            }

            // 2. Direct name or description matching
            if goal_title_lower.contains(&skill_name_clean)
                || skill_name_clean.contains(&goal_title_lower)
            {
                match_score += 0.7;
            } else if goal_desc_lower.contains(&skill_name_clean)
                || skill_desc_clean.contains(&goal_title_lower)
            {
                match_score += 0.4;
            }

            // 3. Action match
            if !goal_action.is_empty()
                && skill
                    .steps
                    .iter()
                    .any(|s| s.tool.to_lowercase() == goal_action)
            {
                match_score += 0.3;
            }

            // 4. Tag overlap bonus
            for tag in &skill.tags {
                let tag_lower = tag.to_lowercase();
                if goal_title_lower.contains(&tag_lower) || goal_desc_lower.contains(&tag_lower) {
                    match_score += 0.2;
                }
            }

            // Quality weighting by success rate and importance
            let total_score = match_score * (skill.success_rate * 0.7 + skill.importance * 0.3);

            if total_score >= 0.5 {
                match best_match {
                    Some((_, prev_score)) if total_score > prev_score => {
                        best_match = Some((skill, total_score));
                    }
                    None => {
                        best_match = Some((skill, total_score));
                    }
                    _ => {}
                }
            }
        }

        best_match.map(|(s, _)| s)
    }

    /// Matches an observed execution error against mined procedural recovery routines.
    pub fn find_recovery_macro_for_error(
        &self,
        error_type: &str,
        error_message: &str,
    ) -> Option<&ProceduralSkill> {
        let err_type_lower = error_type.to_lowercase();
        let err_msg_lower = error_message.to_lowercase();

        let mut candidate_skills: Vec<&ProceduralSkill> = self
            .skills
            .iter()
            .filter(|s| {
                // Must be marked as a recovery skill
                s.tags.iter().any(|t| t == "recovery" || t == "auto_mined")
                    || s.name.starts_with("Recover_")
            })
            .filter(|s| s.success_rate >= self.min_success_rate && !s.steps.is_empty())
            .collect();

        // Sort descending by success rate and step count
        candidate_skills.sort_by(|a, b| {
            b.success_rate
                .partial_cmp(&a.success_rate)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| b.steps.len().cmp(&a.steps.len()))
        });

        for skill in candidate_skills {
            // Check preconditions for error pattern match
            let matches_precondition = skill.preconditions.iter().any(|pre| {
                let pre_lower = pre.to_lowercase();
                pre_lower.contains(&err_type_lower)
                    || (!err_msg_lower.is_empty() && pre_lower.contains(&err_msg_lower))
            });

            // Check tags or name
            let matches_tag = skill
                .tags
                .iter()
                .any(|t| t.to_lowercase() == err_type_lower);
            let matches_name = skill
                .name
                .to_lowercase()
                .contains(&err_type_lower.replace(|c: char| !c.is_alphanumeric(), ""));

            if matches_precondition || matches_tag || matches_name {
                return Some(skill);
            }
        }

        None
    }

    /// Compiles all matching procedural macros into the Goal DAG in-place.
    ///
    /// Replaces matched subgoals with concrete sequential macro action steps,
    /// rewires incoming and outgoing dependency edges, and returns a detailed compilation report.
    pub fn compile_macros(&self, dag: &mut GoalDag) -> Result<MacroCompilationReport> {
        let mut report = MacroCompilationReport::new();

        // Identify candidate nodes to expand (only pending Task nodes)
        let candidate_nodes: Vec<GoalNode> = dag
            .all_nodes()
            .into_iter()
            .filter(|n| {
                n.status == GoalStatus::Pending
                    && matches!(n.kind, GoalNodeKind::Task | GoalNodeKind::Phase)
            })
            .cloned()
            .collect();

        for node in candidate_nodes {
            if let Some(skill) = self.find_best_macro_for_goal(&node) {
                let step_ids = dag.expand_macro(&node.id, skill, self.auto_inject_verification)?;
                if !step_ids.is_empty() {
                    let steps_len = step_ids.len();
                    report.substitutions.push(MacroSubstitution {
                        original_node_id: node.id.clone(),
                        original_title: node.title.clone(),
                        skill_name: skill.name.clone(),
                        skill_id: skill.id,
                        generated_step_ids: step_ids,
                        steps_count: steps_len,
                        success_rate: skill.success_rate,
                    });
                    report.total_steps_compiled += steps_len;
                    report.replanning_bypassed = true;
                }
            }
        }

        if report.has_substitutions() {
            dag.validate()?;
        }

        Ok(report)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::planning::types::GoalEdge;
    use crate::planning::{DagScheduler, DynamicReplanner, RecoveryAction};
    use strata_core::schemas::ProceduralStep;

    fn sample_skill() -> ProceduralSkill {
        let mut skill = ProceduralSkill::new(
            "setup_database",
            "Creates database tables and indexes for auth",
        );
        skill.preconditions = vec!["Task domain: Database Setup".to_string()];
        skill.postconditions = vec!["Tables auth_users and sessions exist".to_string()];
        skill.success_rate = 0.95;
        skill.importance = 0.8;
        skill.tags = vec!["database".to_string(), "setup".to_string()];
        skill.steps = vec![
            ProceduralStep::new(
                1,
                "sqlite",
                "CREATE TABLE auth_users (id TEXT PRIMARY KEY, email TEXT);",
                serde_json::json!({ "mode": "write" }),
            ),
            ProceduralStep::new(
                2,
                "sqlite",
                "CREATE INDEX idx_auth_users_email ON auth_users(email);",
                serde_json::json!({ "mode": "write" }),
            ),
        ];
        skill
    }

    fn sample_recovery_skill() -> ProceduralSkill {
        let mut skill = ProceduralSkill::new(
            "Recover_DeadlockTimeout",
            "Mitigates SQLite deadlock by bumping busy timeout and retrying",
        );
        skill.preconditions = vec!["Error pattern matches 'DeadlockTimeout'".to_string()];
        skill.tags = vec![
            "recovery".to_string(),
            "auto_mined".to_string(),
            "DeadlockTimeout".to_string(),
        ];
        skill.success_rate = 0.90;
        skill.importance = 0.9;
        skill.steps = vec![
            ProceduralStep::new(
                1,
                "sqlite_pragma",
                "PRAGMA busy_timeout = 5000;",
                serde_json::json!({ "timeout": 5000 }),
            ),
            ProceduralStep::new(
                2,
                "transaction_retry",
                "Retry failed transaction block",
                serde_json::json!({ "retry": true }),
            ),
        ];
        skill
    }

    #[test]
    fn test_find_best_macro_for_goal_preconditions() {
        let compiler = ProceduralMacroCompiler::new().with_skills(vec![sample_skill()]);

        let matching_goal = GoalNode::task("goal-1", "Database Setup Task");
        let matched = compiler.find_best_macro_for_goal(&matching_goal);
        assert!(matched.is_some());
        assert_eq!(matched.unwrap().name, "setup_database");

        let unrelated_goal = GoalNode::task("goal-2", "Send Welcome Email");
        let unmatch = compiler.find_best_macro_for_goal(&unrelated_goal);
        assert!(unmatch.is_none());
    }

    #[test]
    fn test_goal_dag_macro_expansion_and_topology() {
        let mut dag = GoalDag::new();
        let n1 = GoalNode::task("step_init", "Initialize environment");
        let n2 = GoalNode::task("step_db", "Database Setup Task");
        let n3 = GoalNode::task("step_server", "Start HTTP server");

        dag.add_node(n1);
        dag.add_node(n2);
        dag.add_node(n3);

        dag.add_edge("step_init", "step_db", GoalEdge::depends_on())
            .unwrap();
        dag.add_edge("step_db", "step_server", GoalEdge::depends_on())
            .unwrap();
        dag.validate().unwrap();

        let compiler = ProceduralMacroCompiler::new().with_skills(vec![sample_skill()]);
        let report = compiler.compile_macros(&mut dag).unwrap();

        assert!(report.has_substitutions());
        assert_eq!(report.substitutions.len(), 1);
        assert_eq!(report.substitutions[0].original_node_id, "step_db");
        assert_eq!(report.substitutions[0].skill_name, "setup_database");
        assert_eq!(report.substitutions[0].steps_count, 3); // 2 steps + 1 verify

        // Validate DAG topology after expansion
        dag.validate().unwrap();

        // Node step_db should be marked skipped
        assert_eq!(dag.get_node("step_db").unwrap().status, GoalStatus::Skipped);

        // Compute execution waves
        let waves = dag.compute_waves().unwrap();
        assert!(!waves.is_empty());

        // Check that initial node executes before macro steps, and macro steps execute before server
        let prereqs_server = dag.get_prerequisites("step_server");
        assert!(prereqs_server.iter().any(|p| p.contains("macro_verify")));
    }

    #[tokio::test]
    async fn test_scheduler_macro_compilation_and_execution() {
        let mut dag = GoalDag::new();
        let task = GoalNode::task("db_setup_goal", "Database Setup Task");
        dag.add_node(task);

        let compiler = ProceduralMacroCompiler::new().with_skills(vec![sample_skill()]);
        let scheduler = DagScheduler::new().with_macro_compiler(compiler);

        let (completed_dag, report) = scheduler.execute(dag).await.unwrap();
        assert_eq!(report.total_nodes, 4); // 1 skipped original + 2 macro tasks + 1 verify
        assert_eq!(report.failed_nodes, 0);

        // Verification node should be completed
        let verify_node = completed_dag
            .all_nodes()
            .into_iter()
            .find(|n| n.id.contains("macro_verify"));
        assert!(verify_node.is_some());
        assert_eq!(verify_node.unwrap().status, GoalStatus::Completed);
    }

    #[test]
    fn test_dynamic_replanner_mined_recovery_routine() {
        let mut dag = GoalDag::new();
        let mut failed_task = GoalNode::task("task_write", "Write transaction batch");
        failed_task.error = Some("DeadlockTimeout: SQLite writer queue saturated".to_string());
        failed_task.retry_count = failed_task.max_retries; // Transient retries exhausted
        dag.add_node(failed_task.clone());

        let replanner =
            DynamicReplanner::new().with_procedural_skills(vec![sample_recovery_skill()]);

        let action = replanner.handle_failure(&dag, &failed_task).unwrap();
        match action {
            RecoveryAction::InjectMitigation {
                failed_node_id,
                mitigation_nodes,
                edges,
            } => {
                assert_eq!(failed_node_id, "task_write");
                assert_eq!(mitigation_nodes.len(), 3); // 2 recovery steps + 1 reverify
                assert_eq!(edges.len(), 2);

                // Apply recovery patch
                replanner
                    .apply_recovery(
                        &mut dag,
                        &RecoveryAction::InjectMitigation {
                            failed_node_id,
                            mitigation_nodes,
                            edges,
                        },
                    )
                    .unwrap();

                // DAG must remain valid and acyclic
                dag.validate().unwrap();
                let waves = dag.compute_waves().unwrap();
                assert_eq!(waves.len(), 3); // step 1 -> step 2 -> reverify
            }
            other => panic!("Expected InjectMitigation, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_macro_execution_engine_1step_success() {
        let engine = MacroExecutionEngine::new();

        let steps = vec![
            MacroStep::new(
                1,
                "read_file",
                "read",
                serde_json::json!({ "path": "{{target_path}}" }),
            ),
            MacroStep::new(
                2,
                "edit_file",
                "edit",
                serde_json::json!({ "path": "{{target_path}}", "content": "{{patch_content}}" }),
            ),
            MacroStep::new(
                3,
                "cargo_test",
                "test",
                serde_json::json!({ "package": "{{pkg_name}}" }),
            ),
        ];

        let parameters = vec![
            MacroParameter::new("target_path", "Target source file", true),
            MacroParameter::new("patch_content", "Replacement code", true),
            MacroParameter::new("pkg_name", "Target crate", false)
                .with_default(serde_json::json!("strata-core")),
        ];

        let compiled_macro = CompiledMacro::new("safe_edit_and_test", "Refactor module and verify")
            .with_steps(steps)
            .with_parameters(parameters);

        let invoker = CallbackToolInvoker::new(|tool: &str, input: &serde_json::Value| {
            if tool == "read_file" {
                assert_eq!(input["path"], "crates/strata-core/src/state.rs");
                Ok(serde_json::json!({ "content": "original content" }))
            } else if tool == "edit_file" {
                assert_eq!(input["path"], "crates/strata-core/src/state.rs");
                assert_eq!(input["content"], "pub fn updated() {}");
                Ok(serde_json::json!({ "modified": true }))
            } else if tool == "cargo_test" {
                assert_eq!(input["package"], "strata-core");
                Ok(serde_json::json!({ "passed": 12, "failed": 0 }))
            } else {
                anyhow::bail!("Unexpected tool: {tool}")
            }
        });

        let mut params = HashMap::new();
        params.insert(
            "target_path".to_string(),
            serde_json::json!("crates/strata-core/src/state.rs"),
        );
        params.insert(
            "patch_content".to_string(),
            serde_json::json!("pub fn updated() {}"),
        );

        let report = engine
            .execute_macro_1step(&compiled_macro, &params, &invoker)
            .await
            .unwrap();

        assert!(report.success);
        assert_eq!(report.steps_completed, 3);
        assert_eq!(report.total_steps, 3);
        assert_eq!(report.tokens_saved, 3 * 450);
        assert_eq!(report.final_output["passed"], 12);
    }

    #[tokio::test]
    async fn test_macro_execution_engine_halts_on_failure() {
        let engine = MacroExecutionEngine::new();

        let steps = vec![
            MacroStep::new(1, "step_1", "act", serde_json::json!({})),
            MacroStep::new(2, "step_2", "act", serde_json::json!({})),
            MacroStep::new(3, "step_3", "act", serde_json::json!({})),
        ];

        let compiled_macro =
            CompiledMacro::new("failing_macro", "Halt on step 2").with_steps(steps);

        let invoker = CallbackToolInvoker::new(|tool: &str, _: &serde_json::Value| {
            if tool == "step_1" {
                Ok(serde_json::json!({ "ok": true }))
            } else if tool == "step_2" {
                anyhow::bail!("Disk full")
            } else {
                panic!("Step 3 should not be executed after step 2 failed!");
            }
        });

        let report = engine
            .execute_macro_1step(&compiled_macro, &HashMap::new(), &invoker)
            .await
            .unwrap();

        assert!(!report.success);
        assert_eq!(report.steps_completed, 1);
        assert!(report.error.unwrap().contains("Disk full"));
    }
}
