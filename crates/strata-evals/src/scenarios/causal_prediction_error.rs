use anyhow::{bail, Result};
use strata_core::schemas::{FactStatus, SemanticFact};
use strata_core::state::Scope;
use strata_memory::jtms::TruthMaintenanceSystem;
use strata_memory::SqliteStore;
use strata_reasoning::{
    ActionExpectation, CausalEdgeKind, CausalNode, CausalNodeKind, CausalPostcondition,
    CausalPrecondition, ObservedOutcome, WorldModel,
};

/// Scenario 29: Causal Prediction-Error Belief Graph & Counterfactual Learning (Phase 4.1)
///
/// Demonstrates closed-loop continual learning:
/// 1. Action fails unexpectedly due to a missing environmental condition.
/// 2. Causal prediction engine calculates discrepancy (prediction error).
/// 3. Counterfactual attribution identifies the pivot variable ("why it failed and what would fix it").
/// 4. Graph of beliefs and JTMS cascade invalidates false assumptions.
/// 5. Counterfactual intervention restores deterministic success.
pub async fn run_causal_prediction_error_scenario() -> Result<()> {
    println!("\n--- [Scenario 29] Causal Prediction-Error & Counterfactual Learning ---");

    // 1. Initialize SQLite store, JTMS and Cognitive World Model
    let store = SqliteStore::open_in_memory()?;
    let jtms = TruthMaintenanceSystem::with_default_threshold();
    let world_model = WorldModel::new();

    // 2. Register existing semantic facts with causal dependency in JTMS
    let mut fact_baseline = SemanticFact::new(
        "cargo test passes in bare workspace mode without extra flags",
        "toolchain",
        Scope::Global,
    );
    fact_baseline.tags = vec!["cargo_test".to_string(), "flag_all_targets".to_string()];
    store.insert_or_update_semantic_fact(&fact_baseline)?;

    let fact_dependent = SemanticFact::new(
        "CI verification step assumes bare cargo test is sufficient",
        "ci",
        Scope::Global,
    )
    .with_dependency(fact_baseline.id);
    store.insert_or_update_semantic_fact(&fact_dependent)?;

    println!("  [1] Seeded baseline beliefs in JTMS (Fact 1: Active, Fact 2: Active)");

    // 3. Register nodes in World Model causal graph
    {
        let mut graph = world_model.read_graph().await.clone();
        graph.add_node(CausalNode::new(
            "cargo_test",
            "Cargo Test Runner",
            CausalNodeKind::Action,
        ));
        graph.add_node(CausalNode::new(
            "flag_all_targets",
            "All Targets Flag",
            CausalNodeKind::ConfigOption,
        ));
    }

    // 4. Formulate ActionExpectation
    let expectation = ActionExpectation::new("cargo_test")
        .with_expected_success(true)
        .with_precondition(CausalPrecondition::new(
            "rust_installed",
            "Rust toolchain available",
        ))
        .with_expected_postcondition(CausalPostcondition::new(
            "test_suite_passed",
            "All integration tests pass",
        ))
        .with_variable("flag_all_targets", serde_json::json!(false))
        .with_variable("target_profile", serde_json::json!("dev"));

    println!("  [2] ActionExpectation formulated: expected_success=true, flag_all_targets=false");

    // 5. Simulate execution failure (divergence from expectation)
    let outcome = ObservedOutcome::failure(
        "error: integration test target failed to link, flag --all-targets required",
        Some(101),
    );

    // 6. Process action feedback: calculate prediction error & perform counterfactual attribution
    let (maybe_error, maybe_attribution, maybe_update) = world_model
        .process_action_feedback(&expectation, &outcome)
        .await?;

    let error = match maybe_error {
        Some(e) => e,
        None => bail!("Prediction error must be detected when action failed unexpectedly"),
    };

    println!(
        "  [3] Prediction Error computed: discrepancy={:.2}, mismatch_type={}",
        error.discrepancy_score, error.mismatch_type
    );
    if error.discrepancy_score < 0.50 {
        bail!("Discrepancy score must be >= 0.50 for unexpected action failure");
    }

    let attribution = match maybe_attribution {
        Some(a) => a,
        None => bail!("Counterfactual attribution must be produced for prediction error"),
    };

    println!(
        "  [4] Counterfactual Attribution: pivot_variable='{}' (confidence={:.1}%)",
        attribution.pivot_variable,
        attribution.attribution_confidence * 100.0
    );
    if attribution.pivot_variable != "flag_all_targets" {
        bail!(
            "Expected pivot variable 'flag_all_targets', got '{}'",
            attribution.pivot_variable
        );
    }

    let update = match maybe_update {
        Some(u) => u,
        None => bail!("Causal belief update must be generated"),
    };

    // 7. Verify Causal Graph updated with learned dependency
    {
        let graph = world_model.read_graph().await;
        let incoming = graph.incoming_dependencies("flag_all_targets");
        let has_dep = incoming
            .iter()
            .any(|(n, e)| n.id == "cargo_test" && e.kind == CausalEdgeKind::DependsOn);
        if !has_dep {
            bail!("CausalGraph must register DependsOn edge from action to pivot variable");
        }
        println!("  [5] CausalGraph updated with learned edge: cargo_test -> flag_all_targets (DependsOn)");
    }

    // 8. Feed CausalBeliefUpdate to JTMS: invalidation cascade
    let invalidated =
        jtms.apply_causal_invalidation_cascade(&store, &update, "Prediction Error Feedback")?;
    if !invalidated.contains(&fact_baseline.id) {
        bail!("Baseline fact must be invalidated by JTMS causal cascade");
    }
    if !invalidated.contains(&fact_dependent.id) {
        bail!("Dependent fact must cascade to Stale via JTMS");
    }

    let fact_1_after = store.get_semantic_fact(&fact_baseline.id)?.unwrap();
    let fact_2_after = store.get_semantic_fact(&fact_dependent.id)?.unwrap();
    if fact_1_after.status != FactStatus::Stale || fact_2_after.status != FactStatus::Stale {
        bail!("Both facts must be Stale after causal invalidation cascade");
    }
    println!("  [6] JTMS Cascade: Fact 1 -> Stale, Fact 2 -> Stale (Audits recorded)");

    // 9. Pre-flight test counterfactual intervention
    let intervention = attribution
        .minimal_sufficient_intervention
        .expect("Minimal intervention must be proposed");

    let sim = world_model
        .simulate_counterfactual("cargo_test", &intervention)
        .await;

    println!(
        "  [7] Counterfactual Simulation: bypasses_failure={}, predicted_success={:.1}%",
        sim.bypasses_failure,
        sim.predicted_success_rate * 100.0
    );
    if !sim.bypasses_failure {
        bail!("Counterfactual intervention should bypass failure");
    }

    // 10. Re-run action with counterfactual intervention applied
    let corrected_expectation = ActionExpectation::new("cargo_test")
        .with_expected_success(true)
        .with_expected_postcondition(CausalPostcondition::new(
            "test_suite_passed",
            "All integration tests pass",
        ))
        .with_variable(
            "flag_all_targets",
            intervention.counterfactual_value.clone(),
        );

    let mut ok_post = CausalPostcondition::new("test_suite_passed", "All integration tests pass");
    ok_post.satisfied = true;
    let successful_outcome = ObservedOutcome::success(serde_json::json!({ "tests_passed": 42 }))
        .with_postconditions(vec![ok_post]);

    let (re_error, _, _) = world_model
        .process_action_feedback(&corrected_expectation, &successful_outcome)
        .await?;

    if re_error.is_some() {
        bail!("Corrected action must have zero prediction error");
    }

    println!("  [8] Re-executed with intervention: 0 prediction error, execution succeeded!");
    println!("✅ Scenario 29 Passed: Closed-loop causal prediction error & counterfactual learning verified.\n");

    Ok(())
}
