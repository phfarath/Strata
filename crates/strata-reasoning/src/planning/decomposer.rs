use std::sync::Arc;
use anyhow::Result;

use crate::engine::{ChatMessage, PromptContext, ReasoningEngine};
use super::dag::GoalDag;
use super::types::{GoalNode, GoalNodeKind};

/// Hierarchical goal decomposer that parses high-level objectives into executable Goal DAGs.
pub struct GoalDecomposer {
    default_max_depth: usize,
    include_verification_gates: bool,
}

impl Default for GoalDecomposer {
    fn default() -> Self {
        Self::new()
    }
}

impl GoalDecomposer {
    pub fn new() -> Self {
        Self {
            default_max_depth: 3,
            include_verification_gates: true,
        }
    }

    pub fn with_max_depth(mut self, depth: usize) -> Self {
        self.default_max_depth = depth;
        self
    }

    pub fn with_verification_gates(mut self, enabled: bool) -> Self {
        self.include_verification_gates = enabled;
        self
    }

    /// Decompose a natural language user objective into a structured `GoalDag`.
    pub fn decompose(&self, goal: &str) -> Result<GoalDag> {
        let trimmed = goal.trim();
        let lower = trimmed.to_lowercase();

        // 1. Check for explicit multi-step instructions (numbered items or semicolons or bullet points)
        if trimmed.contains('\n') || trimmed.contains(';') || trimmed.contains("1.") || trimmed.contains("1)") {
            return self.decompose_multi_step(trimmed);
        }

        // 2. Specialized template for bug fixes and incident recovery
        if lower.starts_with("fix") || lower.contains("debug") || lower.contains("bug") || lower.contains("error") {
            return self.decompose_bugfix(trimmed);
        }

        // 3. Specialized template for refactoring / migrations / feature implementations
        self.decompose_engineering_task(trimmed)
    }

    /// Async decomposition optionally utilizing an LLM ReasoningEngine if provided.
    pub async fn decompose_with_engine(
        &self,
        engine: Option<Arc<dyn ReasoningEngine>>,
        goal: &str,
    ) -> Result<GoalDag> {
        if let Some(engine) = engine {
            let prompt = format!(
                "Decompose this software engineering goal into a structured JSON execution plan with parallel waves:\n\
                Goal: \"{goal}\"\n\
                Return JSON format:\n\
                {{\n  \"nodes\": [\n    {{\"id\": \"task-1\", \"title\": \"...\", \"kind\": \"task|verification|phase\"}}\n  ],\n  \"dependencies\": [\n    {{\"dependent\": \"task-2\", \"prerequisite\": \"task-1\"}}\n  ]\n}}"
            );

            let ctx = PromptContext::new().with_message(ChatMessage::user(prompt));
            if let Ok(output) = engine.complete(&ctx).await {
                if let Some(content) = output.content {
                    if let Ok(dag) = self.parse_json_dag(&content) {
                        return Ok(dag);
                    }
                }
            }
        }

        // Fallback to deterministic template decomposition
        self.decompose(goal)
    }

    fn decompose_engineering_task(&self, goal: &str) -> Result<GoalDag> {
        let mut dag = GoalDag::new();

        // Root node
        let root = GoalNode::root("root_goal", goal)
            .with_description(format!("Execute long-horizon objective: '{goal}'"));
        dag.add_node(root);

        // Phase 1: Architectural Analysis & Invariant Check (Wave 0)
        let analyze_task = GoalNode::task(
            "analyze_architecture",
            "Analyze architecture, causal blast radius & contract invariants",
        )
        .with_description("Evaluate code dependencies and identify critical invariants before applying changes")
        .with_action("strata blast-radius --depth 3");

        let prepare_env_task = GoalNode::task(
            "prepare_test_fixtures",
            "Prepare test fixtures & sandbox environment",
        )
        .with_description("Initialize mock stores, seed test records, and ensure isolation");

        dag.add_node(analyze_task);
        dag.add_node(prepare_env_task);

        // Phase 2: Core Implementation & Integration (Wave 1)
        let implement_core = GoalNode::task(
            "implement_core_logic",
            format!("Implement core changes for '{goal}'"),
        )
        .with_description("Apply required code modifications, type definitions, and algorithmic routines");

        let integrate_adapters = GoalNode::task(
            "integrate_adapters",
            "Wire system adapters & tool integrations",
        )
        .with_description("Expose new capabilities to tools gateway, MCP server, and CLI commands");

        dag.add_node(implement_core);
        dag.add_node(integrate_adapters);

        // Prerequisite dependencies: Wave 0 -> Wave 1
        dag.add_dependency("implement_core_logic", "analyze_architecture")?;
        dag.add_dependency("implement_core_logic", "prepare_test_fixtures")?;
        dag.add_dependency("integrate_adapters", "analyze_architecture")?;

        // Phase 3: Verification Gates (Wave 2)
        if self.include_verification_gates {
            let verify_invariants = GoalNode::verification(
                "verify_contract_invariants",
                "Verify contract invariants & pass unit test suite",
            )
            .with_description("Run cargo test --workspace and check semantic invariant assertions")
            .with_action("cargo test --workspace");

            dag.add_node(verify_invariants);
            dag.add_dependency("verify_contract_invariants", "implement_core_logic")?;
            dag.add_dependency("verify_contract_invariants", "integrate_adapters")?;

            // Phase 4: Consolidation & Documentation (Wave 3)
            let consolidate_task = GoalNode::task(
                "consolidate_documentation",
                "Consolidate architecture documentation & memory digest",
            )
            .with_description("Record durable decision takeaways, update AGENTS.md, and record procedural skills");

            dag.add_node(consolidate_task);
            dag.add_dependency("consolidate_documentation", "verify_contract_invariants")?;
        } else {
            let consolidate_task = GoalNode::task(
                "consolidate_documentation",
                "Consolidate architecture documentation & memory digest",
            );
            dag.add_node(consolidate_task);
            dag.add_dependency("consolidate_documentation", "implement_core_logic")?;
            dag.add_dependency("consolidate_documentation", "integrate_adapters")?;
        }

        Ok(dag)
    }

    fn decompose_bugfix(&self, goal: &str) -> Result<GoalDag> {
        let mut dag = GoalDag::new();

        let root = GoalNode::root("root_goal", goal);
        dag.add_node(root);

        // Wave 0: Reproduction
        let repro = GoalNode::task(
            "reproduce_and_diagnose",
            format!("Reproduce failure & isolate root cause for '{goal}'"),
        );
        dag.add_node(repro);

        // Wave 1: Targeted Patch
        let patch = GoalNode::task(
            "implement_targeted_patch",
            "Implement minimal targeted patch & update defensive assertions",
        );
        dag.add_node(patch);
        dag.add_dependency("implement_targeted_patch", "reproduce_and_diagnose")?;

        // Wave 2: Regression Verification Gate
        let verify = GoalNode::verification(
            "verify_regression_tests",
            "Verify bug is resolved and no regressions introduced",
        );
        dag.add_node(verify);
        dag.add_dependency("verify_regression_tests", "implement_targeted_patch")?;

        // Wave 3: Failure Anti-Pattern Capture
        let anti_pattern = GoalNode::task(
            "record_failure_anti_pattern",
            "Record failure signature & mitigation pattern into Strata persistent memory",
        );
        dag.add_node(anti_pattern);
        dag.add_dependency("record_failure_anti_pattern", "verify_regression_tests")?;

        Ok(dag)
    }

    fn decompose_multi_step(&self, text: &str) -> Result<GoalDag> {
        let mut dag = GoalDag::new();

        let lines: Vec<&str> = text
            .split(&['\n', ';'][..])
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .collect();

        let root = GoalNode::root("root_goal", lines.first().copied().unwrap_or("Multi-step Goal"));
        dag.add_node(root);

        let mut prev_task_id: Option<String> = None;

        for (i, line) in lines.iter().enumerate() {
            let clean_line = line
                .trim_start_matches(|c: char| c.is_numeric() || c == '.' || c == ')' || c == '-' || c == '*' || c == ' ');

            let task_id = format!("step_{}", i + 1);
            let node = GoalNode::task(&task_id, clean_line);
            dag.add_node(node);

            if let Some(prev) = prev_task_id {
                dag.add_dependency(&task_id, &prev)?;
            }
            prev_task_id = Some(task_id);
        }

        if self.include_verification_gates {
            if let Some(last_id) = prev_task_id {
                let verify = GoalNode::verification(
                    "verify_multi_step_completion",
                    "Verify all execution steps and acceptance criteria",
                );
                dag.add_node(verify);
                dag.add_dependency("verify_multi_step_completion", &last_id)?;
            }
        }

        Ok(dag)
    }

    fn parse_json_dag(&self, json_str: &str) -> Result<GoalDag> {
        // Strip markdown code fences if LLM returns ```json
        let clean = json_str
            .trim()
            .trim_start_matches("```json")
            .trim_start_matches("```")
            .trim_end_matches("```")
            .trim();

        let val: serde_json::Value = serde_json::from_str(clean)?;
        let mut dag = GoalDag::new();

        if let Some(nodes) = val.get("nodes").and_then(|v| v.as_array()) {
            for n in nodes {
                let id = n.get("id").and_then(|v| v.as_str()).unwrap_or("task");
                let title = n.get("title").and_then(|v| v.as_str()).unwrap_or("Task");
                let kind_str = n.get("kind").and_then(|v| v.as_str()).unwrap_or("task");

                let kind = match kind_str {
                    "verification" => GoalNodeKind::Verification,
                    "phase" => GoalNodeKind::Phase,
                    "rollback" => GoalNodeKind::Rollback,
                    "root" => GoalNodeKind::Root,
                    _ => GoalNodeKind::Task,
                };

                let node = GoalNode::new(id, title, kind);
                dag.add_node(node);
            }
        }

        if let Some(deps) = val.get("dependencies").and_then(|v| v.as_array()) {
            for d in deps {
                let dep = d.get("dependent").and_then(|v| v.as_str());
                let prereq = d.get("prerequisite").and_then(|v| v.as_str());
                if let (Some(dep_id), Some(prereq_id)) = (dep, prereq) {
                    if dag.contains_node(dep_id) && dag.contains_node(prereq_id) {
                        let _ = dag.add_dependency(dep_id, prereq_id);
                    }
                }
            }
        }

        dag.validate()?;
        Ok(dag)
    }
}
