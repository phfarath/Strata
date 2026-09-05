use anyhow::{bail, Result};
use strata_reasoning::{
    CausalEdge, CausalEdgeKind, CausalGraph, CausalNode, CausalNodeKind, WorldModel,
};

/// Scenario 10: World Model, Causal Architecture Graph & Blast Radius Prediction
/// Evaluates:
/// 1. Dynamic CausalGraph construction with petgraph DiGraph in Rust.
/// 2. Direct vs. transitive blast radius traversal and coupling propagation.
/// 3. Breaking change risk detection and architectural contract invariant enforcement.
/// 4. Pre-flight patch simulation across multi-file modifications.
pub async fn run_world_model_causal_scenario() -> Result<()> {
    println!("\n▶ Running Eval Scenario: World Model & Dynamic Causal Architecture Graph");

    // 1. Initialize Causal Graph and seed nodes
    let mut graph = CausalGraph::new();

    let storage_file = CausalNode::new(
        "file:crates/strata-server/src/storage.rs",
        "storage.rs",
        CausalNodeKind::File,
    )
    .with_path("crates/strata-server/src/storage.rs");
    graph.add_node(storage_file);

    let storage_struct = CausalNode::new(
        "struct:ServerStorage",
        "ServerStorage",
        CausalNodeKind::Struct,
    )
    .with_path("crates/strata-server/src/storage.rs");
    graph.add_node(storage_struct);

    let handlers_module = CausalNode::new(
        "module:strata_server::handlers",
        "handlers.rs",
        CausalNodeKind::Module,
    )
    .with_path("crates/strata-server/src/handlers.rs");
    graph.add_node(handlers_module);

    let push_api = CausalNode::new(
        "endpoint:POST /api/v1/sync/push",
        "API: POST /api/v1/sync/push",
        CausalNodeKind::Endpoint,
    );
    graph.add_node(push_api);

    let outbox_table = CausalNode::new(
        "table:sync_outbox",
        "Database Table: sync_outbox",
        CausalNodeKind::DatabaseTable,
    );
    graph.add_node(outbox_table);

    let invariant_node = CausalNode::new(
        "invariant:offline_first_cdc_sequence",
        "Invariant: Monotonic CDC Outbox Sequence",
        CausalNodeKind::ContractInvariant,
    );
    graph.add_node(invariant_node);

    // 2. Add Causal Directed Edges (Consumer/Caller -> Dependency)
    // storage.rs extends ServerStorage
    graph.add_edge(
        "file:crates/strata-server/src/storage.rs",
        "struct:ServerStorage",
        CausalEdge::new(CausalEdgeKind::Extends, 1.0, true),
    )?;

    // ServerStorage writes to sync_outbox table
    graph.add_edge(
        "struct:ServerStorage",
        "table:sync_outbox",
        CausalEdge::writes_to(1.0),
    )?;

    // handlers.rs calls ServerStorage::push_deltas
    graph.add_edge(
        "module:strata_server::handlers",
        "struct:ServerStorage",
        CausalEdge::calls(0.9, true),
    )?;

    // POST /api/v1/sync/push endpoint exposes handlers.rs
    graph.add_edge(
        "endpoint:POST /api/v1/sync/push",
        "module:strata_server::handlers",
        CausalEdge::exposes_endpoint(1.0),
    )?;

    // Invariant contract enforces rules on sync_outbox table
    graph.add_edge(
        "invariant:offline_first_cdc_sequence",
        "table:sync_outbox",
        CausalEdge::enforces_contract(1.0)
            .with_description("CDC outbox must maintain monotonic seq numbering"),
    )?;

    println!("  [Causal Graph Metrics]");
    println!("    • Total Nodes in Graph: {}", graph.node_count());
    println!("    • Total Directed Edges: {}", graph.edge_count());

    if graph.node_count() < 6 {
        bail!(
            "Expected at least 6 nodes in graph, got {}",
            graph.node_count()
        );
    }

    if graph.edge_count() < 5 {
        bail!(
            "Expected at least 5 edges in graph, got {}",
            graph.edge_count()
        );
    }

    // 3. Compute Blast Radius for DB Table `table:sync_outbox`
    let blast_report = graph.compute_blast_radius("table:sync_outbox", 3);

    println!("  [Blast Radius Analysis: table:sync_outbox]");
    println!("    • Target:                {}", blast_report.target_name);
    println!(
        "    • Direct Dependents:     {}",
        blast_report.direct_impacts.len()
    );
    println!(
        "    • Transitive Dependents: {}",
        blast_report.transitive_impacts.len()
    );
    println!(
        "    • Invariants Triggered:  {}",
        blast_report.triggered_invariants.len()
    );
    println!(
        "    • Pre-Code Risk Score:   {:.1}%",
        blast_report.overall_risk_score * 100.0
    );

    // Direct dependent must be ServerStorage
    let has_direct_storage = blast_report
        .direct_impacts
        .iter()
        .any(|n| n.node_id == "struct:ServerStorage");
    if !has_direct_storage {
        bail!("Expected 'struct:ServerStorage' as direct d=1 dependent of table:sync_outbox");
    }

    // Transitive dependent must be handlers.rs (d=2) and POST /api/v1/sync/push (d=3)
    let has_handlers = blast_report
        .transitive_impacts
        .iter()
        .any(|n| n.node_id == "module:strata_server::handlers" && n.distance == 2);
    if !has_handlers {
        bail!("Expected 'module:strata_server::handlers' as d=2 transitive dependent");
    }

    let has_endpoint = blast_report
        .transitive_impacts
        .iter()
        .any(|n| n.node_id == "endpoint:POST /api/v1/sync/push" && n.distance == 3);
    if !has_endpoint {
        bail!("Expected 'endpoint:POST /api/v1/sync/push' as d=3 transitive dependent");
    }

    // Invariant must be triggered
    if blast_report.triggered_invariants.is_empty() {
        bail!("Expected architectural contract invariant to be triggered on table:sync_outbox");
    }

    if blast_report.overall_risk_score < 0.5 {
        bail!(
            "Expected elevated risk score (>= 50%) for critical table change, got {:.2}",
            blast_report.overall_risk_score
        );
    }

    // 4. Test ASCII Tree Rendering
    let ascii_tree = graph.to_ascii_tree("table:sync_outbox", 3);
    if !ascii_tree.contains("ServerStorage") || !ascii_tree.contains("handlers.rs") {
        bail!("ASCII tree output missing expected nodes");
    }

    // 5. Test WorldModel Integration and Multi-File Patch Simulation
    let world_model = WorldModel::with_graph(graph);
    let patch_sim = world_model
        .simulate_patch(&[
            "file:crates/strata-server/src/storage.rs".to_string(),
            "table:sync_outbox".to_string(),
        ])
        .await?;

    println!("  [Pre-Flight Patch Simulation]");
    println!(
        "    • Modified Targets:      {}",
        patch_sim.modified_targets.len()
    );
    println!(
        "    • Total Impacted Nodes:  {}",
        patch_sim.total_impacted_nodes
    );
    println!(
        "    • Breaking Risks Count:  {}",
        patch_sim.breaking_risks_count
    );
    println!(
        "    • Peak Risk Score:       {:.1}%",
        patch_sim.highest_risk_score * 100.0
    );

    if patch_sim.total_impacted_nodes < 2 {
        bail!(
            "Expected at least 2 impacted nodes in patch simulation, found {}",
            patch_sim.total_impacted_nodes
        );
    }

    println!("  ✓ Petgraph DiGraph causal topology representation verified");
    println!("  ✓ Direct and transitive (d=1, d=2, d=3) ripple propagation verified");
    println!("  ✓ Contract invariants & breaking change risk scoring verified");
    println!("  ✓ Multi-target pre-flight patch simulation verified");

    Ok(())
}
