use chrono::Utc;
use std::collections::HashMap;
use uuid::Uuid;

use super::graph::CausalGraph;
use super::types::{
    ActionExpectation, CausalBeliefUpdate, CausalEdge, CausalNodeKind, CounterfactualAttribution,
    CounterfactualHypothesis, CounterfactualSimulationResult, ObservedOutcome, PredictionError,
    PredictionMismatchType,
};

/// Motor de Causal Prediction-Error e Atribuição Contrafactual (Phase 4.1).
///
/// Modela precondições -> ação -> pós-condições esperadas. Se o outcome divergir,
/// calcula a discrepância (prediction error), identifica a variável pivô via raciocínio
/// contrafactual ("o que aconteceria se a variável X fosse diferente?"), e sintetiza
/// atualizações causais para o Grafo de Crenças e o JTMS.
#[derive(Debug, Clone, Default)]
pub struct CausalPredictionEngine;

impl CausalPredictionEngine {
    pub fn new() -> Self {
        Self
    }

    /// Predicts action feasibility and expected success rate prior to execution using the causal graph.
    pub fn predict_action(
        &self,
        expectation: &ActionExpectation,
        graph: &CausalGraph,
    ) -> (bool, f32, Vec<String>) {
        let mut warnings = Vec::new();
        let mut confidence: f32 = 1.0;

        // 1. Check known preconditions
        for pre in &expectation.preconditions {
            if !pre.satisfied {
                warnings.push(format!("Precondition '{}' is unsatisfied", pre.description));
                confidence *= 0.2;
            }
        }

        // 2. Check graph invariants and anti-patterns
        let blast = graph.compute_blast_radius(&expectation.action_id, 2);
        if !blast.triggered_anti_patterns.is_empty() {
            for ap in &blast.triggered_anti_patterns {
                warnings.push(format!("Active anti-pattern detected: {ap}"));
            }
            confidence *= 0.3;
        }

        if !blast.triggered_invariants.is_empty() {
            for inv in &blast.triggered_invariants {
                warnings.push(format!("Contract invariant warning: {inv}"));
            }
            confidence *= 0.7;
        }

        let predicted_success = confidence >= 0.5 && expectation.expected_success;
        (predicted_success, confidence.clamp(0.0, 1.0), warnings)
    }

    /// Evaluates expected vs observed outcome and calculates prediction error and discrepancy score.
    pub fn calculate_prediction_error(
        &self,
        expectation: &ActionExpectation,
        outcome: &ObservedOutcome,
    ) -> Option<PredictionError> {
        let mut unmet_postconditions = Vec::new();
        let mut violated_preconditions = Vec::new();

        // 1. Check preconditions
        for pre in &expectation.preconditions {
            if !pre.satisfied {
                violated_preconditions.push(pre.clone());
            }
        }

        // 2. Check postconditions
        for expected in &expectation.expected_postconditions {
            let mut satisfied = false;
            for observed in &outcome.observed_postconditions {
                if observed.id == expected.id && observed.satisfied {
                    satisfied = true;
                    break;
                }
            }
            if !satisfied {
                let mut unmet = expected.clone();
                unmet.satisfied = false;
                unmet_postconditions.push(unmet);
            }
        }

        // 3. Determine mismatch type
        let mismatch_type = if expectation.expected_success && !outcome.success {
            if !violated_preconditions.is_empty() {
                PredictionMismatchType::UnsatisfiedPrecondition
            } else {
                PredictionMismatchType::UnexpectedFailure
            }
        } else if !expectation.expected_success && outcome.success {
            PredictionMismatchType::UnexpectedSuccess
        } else if outcome.success && !unmet_postconditions.is_empty() {
            PredictionMismatchType::PostconditionViolation
        } else if !outcome.success && outcome.exit_code == Some(101) {
            PredictionMismatchType::InvariantBreach
        } else if !violated_preconditions.is_empty() {
            PredictionMismatchType::UnsatisfiedPrecondition
        } else if !unmet_postconditions.is_empty() {
            PredictionMismatchType::PostconditionViolation
        } else {
            // Perfect match, no prediction error
            return None;
        };

        // 4. Calculate discrepancy score [0.0, 1.0]
        let mut discrepancy: f32 = 0.0;
        if expectation.expected_success != outcome.success {
            discrepancy += 0.50;
        }

        if !expectation.expected_postconditions.is_empty() {
            let post_ratio = unmet_postconditions.len() as f32
                / expectation.expected_postconditions.len() as f32;
            discrepancy += post_ratio * 0.35;
        } else if !unmet_postconditions.is_empty() {
            discrepancy += 0.35;
        }

        if !expectation.preconditions.is_empty() {
            let pre_ratio =
                violated_preconditions.len() as f32 / expectation.preconditions.len() as f32;
            discrepancy += pre_ratio * 0.15;
        } else if !violated_preconditions.is_empty() {
            discrepancy += 0.15;
        }

        let discrepancy_score = discrepancy.clamp(0.05, 1.0);
        let error_details = outcome
            .error_message
            .clone()
            .unwrap_or_else(|| format!("Prediction mismatch: {mismatch_type}"));

        Some(PredictionError {
            id: format!("pe:{}", Uuid::new_v4().simple()),
            action_id: expectation.action_id.clone(),
            mismatch_type,
            discrepancy_score,
            unmet_postconditions,
            violated_preconditions,
            error_details,
            timestamp: Utc::now(),
        })
    }

    /// Performs counterfactual attribution: evaluates what variable if altered would have prevented failure.
    pub fn perform_counterfactual_attribution(
        &self,
        error: &PredictionError,
        expectation: &ActionExpectation,
        _outcome: &ObservedOutcome,
        graph: &CausalGraph,
    ) -> CounterfactualAttribution {
        let mut hypotheses = Vec::new();
        let err_lower = error.error_details.to_lowercase();

        // 1. Evaluate context variables
        for (var_name, var_val) in &expectation.context_variables {
            let (counterfactual_val, intervention_desc, base_reversal) = match var_val {
                serde_json::Value::Bool(b) => {
                    let inverted = !b;
                    (
                        serde_json::json!(inverted),
                        format!("Toggle boolean variable '{var_name}' from {b} to {inverted}"),
                        0.55,
                    )
                }
                serde_json::Value::String(s) => (
                    serde_json::json!(format!("counterfactual_{s}")),
                    format!("Modify string parameter '{var_name}' from '{s}'"),
                    0.45,
                ),
                serde_json::Value::Null => (
                    serde_json::json!(true),
                    format!("Provide non-null assignment for '{var_name}'"),
                    0.60,
                ),
                _ => (
                    serde_json::json!({ "intervened": true }),
                    format!("Mutate context variable '{var_name}'"),
                    0.40,
                ),
            };

            let mut reversal_score: f32 = base_reversal;

            // Text match boost: error message mentions the variable name, kebab-case, or tokens
            let var_lower = var_name.to_lowercase();
            let var_kebab = var_lower.replace('_', "-");
            let var_snake = var_lower.replace('-', "_");
            let var_words = var_lower.replace(['_', '-'], " ");

            let matches_var = err_lower.contains(&var_lower)
                || err_lower.contains(&var_kebab)
                || err_lower.contains(&var_snake)
                || err_lower.contains(&var_words)
                || var_lower
                    .split(['_', '-'])
                    .filter(|s| s.len() > 3 && *s != "flag")
                    .all(|part| err_lower.contains(part));

            if matches_var {
                reversal_score += 0.35;
            }

            // Graph coupling boost: check if variable node exists in causal graph
            if let Some(node) = graph.get_node(var_name) {
                reversal_score += node.confidence * 0.15;
            }

            let mechanism = format!(
                "Altering '{var_name}' changes environmental precondition for action '{}'",
                expectation.action_id
            );

            hypotheses.push(CounterfactualHypothesis {
                variable_name: var_name.clone(),
                observed_value: var_val.clone(),
                counterfactual_value: counterfactual_val,
                intervention_description: intervention_desc,
                estimated_reversal_probability: reversal_score.clamp(0.1, 0.98),
                causal_mechanism: mechanism,
            });
        }

        // 2. Evaluate violated preconditions
        for pre in &error.violated_preconditions {
            let mechanism = format!(
                "Directly satisfying prerequisite '{}' satisfies action requirement",
                pre.id
            );

            hypotheses.push(CounterfactualHypothesis {
                variable_name: pre.id.clone(),
                observed_value: serde_json::json!(false),
                counterfactual_value: serde_json::json!(true),
                intervention_description: format!("Fulfill precondition: {}", pre.description),
                estimated_reversal_probability: 0.92,
                causal_mechanism: mechanism,
            });
        }

        // 3. Fallback extraction from error text if no hypotheses yet
        if hypotheses.is_empty() {
            let extracted_pivot = if err_lower.contains("flag") || err_lower.contains("--") {
                "cli_flag_missing"
            } else if err_lower.contains("table") || err_lower.contains("relation") {
                "db_migration_state"
            } else if err_lower.contains("permission") || err_lower.contains("lease") {
                "concurrency_lease_acquired"
            } else if err_lower.contains("file") || err_lower.contains("not found") {
                "filesystem_path_exists"
            } else {
                "action_parameter_configuration"
            };

            hypotheses.push(CounterfactualHypothesis {
                variable_name: extracted_pivot.to_string(),
                observed_value: serde_json::json!(false),
                counterfactual_value: serde_json::json!(true),
                intervention_description: format!(
                    "Synthesized intervention for root cause '{}' based on error output",
                    extracted_pivot
                ),
                estimated_reversal_probability: 0.85,
                causal_mechanism: format!(
                    "Extracted from error output cues: '{}'",
                    error.error_details
                ),
            });
        }

        // 4. Rank hypotheses by reversal probability
        hypotheses.sort_by(|a, b| {
            b.estimated_reversal_probability
                .partial_cmp(&a.estimated_reversal_probability)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        let top = &hypotheses[0];
        let pivot_variable = top.variable_name.clone();
        let confidence = top.estimated_reversal_probability;

        let minimal_intervention = hypotheses
            .iter()
            .find(|h| h.estimated_reversal_probability >= 0.70)
            .cloned()
            .or_else(|| hypotheses.first().cloned());

        let root_cause = format!(
            "Action '{}' failed with discrepancy {:.2} ({}). Pivot variable identified: '{}' with {:.1}% reversal probability. Mechanism: {}",
            error.action_id,
            error.discrepancy_score,
            error.mismatch_type,
            pivot_variable,
            confidence * 100.0,
            top.causal_mechanism
        );

        CounterfactualAttribution {
            error_id: error.id.clone(),
            action_id: error.action_id.clone(),
            pivot_variable,
            attribution_confidence: confidence,
            hypotheses,
            minimal_sufficient_intervention: minimal_intervention,
            root_cause_explanation: root_cause,
        }
    }

    /// Synthesizes dynamic belief updates to feed into the CausalGraph and JTMS.
    pub fn synthesize_belief_update(
        &self,
        attribution: &CounterfactualAttribution,
        error: &PredictionError,
        graph: &CausalGraph,
    ) -> CausalBeliefUpdate {
        let mut invalidated_assumptions = Vec::new();
        let mut new_causal_edges = Vec::new();
        let mut node_confidence_penalties = HashMap::new();
        let mut learned_invariants = Vec::new();

        // 1. Invalidate naive assumption that action could succeed without the pivot variable
        let assumption_id = format!(
            "assumption:{}:{}",
            error.action_id, attribution.pivot_variable
        );
        invalidated_assumptions.push(assumption_id.clone());

        // Also check if any existing Assumption nodes in graph connect to this action
        for node in graph.all_nodes() {
            if node.kind == CausalNodeKind::Assumption && node.id.contains(&error.action_id) {
                invalidated_assumptions.push(node.id.clone());
            }
        }

        // 2. Add new causal dependency edge: action depends on pivot variable
        let edge =
            CausalEdge::depends_on(attribution.attribution_confidence).with_description(format!(
                "Learned dependency: action '{}' requires '{}' (attribution confidence {:.2})",
                error.action_id, attribution.pivot_variable, attribution.attribution_confidence
            ));
        new_causal_edges.push((
            error.action_id.clone(),
            attribution.pivot_variable.clone(),
            edge,
        ));

        // 3. Penalize confidence of action or nodes that falsely predicted success
        let penalty = (error.discrepancy_score * 0.35).clamp(0.05, 0.50);
        node_confidence_penalties.insert(error.action_id.clone(), penalty);
        node_confidence_penalties.insert(assumption_id, 0.80);

        // 4. Formulate learned invariant
        if let Some(intervention) = &attribution.minimal_sufficient_intervention {
            learned_invariants.push(format!(
                "Action '{}' requires '{}' = {} ({})",
                error.action_id,
                intervention.variable_name,
                intervention.counterfactual_value,
                intervention.intervention_description
            ));
        }

        CausalBeliefUpdate {
            action_id: error.action_id.clone(),
            invalidated_assumptions,
            new_causal_edges,
            node_confidence_penalties,
            learned_invariants,
        }
    }

    /// Simulates whether a counterfactual intervention would resolve the prediction error.
    pub fn simulate_counterfactual(
        &self,
        action_id: &str,
        intervention: &CounterfactualHypothesis,
        _graph: &CausalGraph,
    ) -> CounterfactualSimulationResult {
        let predicted_success_rate = intervention.estimated_reversal_probability;
        let bypasses_failure = predicted_success_rate >= 0.70;

        let rationale = if bypasses_failure {
            format!(
                "Counterfactual intervention '{}' on variable '{}' is predicted to restore success with {:.1}% confidence.",
                intervention.intervention_description,
                intervention.variable_name,
                predicted_success_rate * 100.0
            )
        } else {
            format!(
                "Counterfactual intervention '{}' only provides {:.1}% reversal probability (below 70% threshold). Additional variables required.",
                intervention.intervention_description,
                predicted_success_rate * 100.0
            )
        };

        CounterfactualSimulationResult {
            action_id: action_id.to_string(),
            tested_intervention: intervention.clone(),
            predicted_success_rate,
            bypasses_failure,
            rationale,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::causal::types::{
        CausalEdgeKind, CausalNode, CausalNodeKind, CausalPostcondition, CausalPrecondition,
    };

    #[test]
    fn test_perfect_prediction_returns_no_error() {
        let engine = CausalPredictionEngine::new();
        let expectation = ActionExpectation::new("cargo_check").with_expected_success(true);
        let outcome = ObservedOutcome::success(serde_json::json!({ "status": "ok" }));

        let error = engine.calculate_prediction_error(&expectation, &outcome);
        assert!(
            error.is_none(),
            "Matching expectation and outcome must produce no error"
        );
    }

    #[test]
    fn test_unexpected_failure_calculates_prediction_error() {
        let engine = CausalPredictionEngine::new();
        let expectation = ActionExpectation::new("cargo_build")
            .with_expected_success(true)
            .with_expected_postcondition(CausalPostcondition::new(
                "target_binary",
                "Binary created",
            ));

        let outcome = ObservedOutcome::failure("error[E0432]: unresolved import", Some(101));

        let error = engine.calculate_prediction_error(&expectation, &outcome);
        assert!(error.is_some());
        let pe = error.unwrap();
        assert_eq!(pe.action_id, "cargo_build");
        assert_eq!(pe.mismatch_type, PredictionMismatchType::UnexpectedFailure);
        assert!(pe.discrepancy_score >= 0.50);
        assert_eq!(pe.unmet_postconditions.len(), 1);
    }

    #[test]
    fn test_unsatisfied_precondition_detected() {
        let engine = CausalPredictionEngine::new();
        let mut pre = CausalPrecondition::new("db_ready", "Postgres DB is running");
        pre.satisfied = false;

        let expectation = ActionExpectation::new("run_migrations")
            .with_expected_success(true)
            .with_precondition(pre);

        let outcome = ObservedOutcome::failure("Connection refused", Some(1));

        let error = engine.calculate_prediction_error(&expectation, &outcome);
        assert!(error.is_some());
        let pe = error.unwrap();
        assert_eq!(
            pe.mismatch_type,
            PredictionMismatchType::UnsatisfiedPrecondition
        );
        assert_eq!(pe.violated_preconditions.len(), 1);
        assert_eq!(pe.violated_preconditions[0].id, "db_ready");
    }

    #[test]
    fn test_postcondition_violation_when_success_reported_but_contract_violated() {
        let engine = CausalPredictionEngine::new();
        let expectation = ActionExpectation::new("generate_docs")
            .with_expected_success(true)
            .with_expected_postcondition(CausalPostcondition::new(
                "index_html",
                "HTML index file created",
            ));

        let outcome = ObservedOutcome::success(serde_json::json!({ "exit": 0 }));

        let error = engine.calculate_prediction_error(&expectation, &outcome);
        assert!(error.is_some());
        let pe = error.unwrap();
        assert_eq!(
            pe.mismatch_type,
            PredictionMismatchType::PostconditionViolation
        );
    }

    #[test]
    fn test_counterfactual_attribution_isolates_pivot_variable() {
        let engine = CausalPredictionEngine::new();
        let mut graph = CausalGraph::new();
        graph.add_node(CausalNode::new(
            "flag_all_targets",
            "All Targets Flag",
            CausalNodeKind::ConfigOption,
        ));

        let expectation = ActionExpectation::new("cargo_test")
            .with_expected_success(true)
            .with_variable("flag_all_targets", serde_json::json!(false))
            .with_variable("opt_level", serde_json::json!(3))
            .with_variable("working_dir", serde_json::json!("."));

        let outcome = ObservedOutcome::failure(
            "error: test binary missing, flag --all-targets required",
            Some(1),
        );

        let error = engine
            .calculate_prediction_error(&expectation, &outcome)
            .expect("prediction error");
        let attribution =
            engine.perform_counterfactual_attribution(&error, &expectation, &outcome, &graph);

        assert_eq!(attribution.pivot_variable, "flag_all_targets");
        assert!(attribution.attribution_confidence > 0.80);
        assert!(attribution.minimal_sufficient_intervention.is_some());
        let intervention = attribution.minimal_sufficient_intervention.unwrap();
        assert_eq!(intervention.counterfactual_value, serde_json::json!(true));
    }

    #[test]
    fn test_belief_update_synthesis_and_graph_mutation() {
        let engine = CausalPredictionEngine::new();
        let mut graph = CausalGraph::new();
        let action_node = CausalNode::new("cargo_build", "Build Tool", CausalNodeKind::Action);
        graph.add_node(action_node);

        let expectation = ActionExpectation::new("cargo_build")
            .with_expected_success(true)
            .with_variable("feature_ssl", serde_json::json!(false));

        let outcome =
            ObservedOutcome::failure("openssl crate missing: enable feature_ssl", Some(1));
        let error = engine
            .calculate_prediction_error(&expectation, &outcome)
            .unwrap();
        let attribution =
            engine.perform_counterfactual_attribution(&error, &expectation, &outcome, &graph);
        let update = engine.synthesize_belief_update(&attribution, &error, &graph);

        assert!(!update.invalidated_assumptions.is_empty());
        assert!(!update.new_causal_edges.is_empty());
        assert!(!update.learned_invariants.is_empty());

        let affected = graph
            .apply_belief_update(&update)
            .expect("apply belief update");
        assert!(!affected.is_empty());

        let target_node = graph.get_node("feature_ssl");
        assert!(
            target_node.is_some(),
            "Pivot variable node must be created in graph"
        );

        let incoming = graph.incoming_dependencies("feature_ssl");
        assert!(incoming
            .iter()
            .any(|(n, e)| n.id == "cargo_build" && e.kind == CausalEdgeKind::DependsOn));
    }

    #[test]
    fn test_counterfactual_simulation_result() {
        let engine = CausalPredictionEngine::new();
        let graph = CausalGraph::new();

        let hypothesis = CounterfactualHypothesis {
            variable_name: "flag_no_std".to_string(),
            observed_value: serde_json::json!(true),
            counterfactual_value: serde_json::json!(false),
            intervention_description: "Disable no_std flag".to_string(),
            estimated_reversal_probability: 0.88,
            causal_mechanism: "Restores std library allocators".to_string(),
        };

        let sim = engine.simulate_counterfactual("build_wasm", &hypothesis, &graph);
        assert!(sim.bypasses_failure);
        assert_eq!(sim.predicted_success_rate, 0.88);
    }
}
