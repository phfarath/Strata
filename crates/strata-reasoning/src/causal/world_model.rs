use std::path::Path;
use std::sync::Arc;
use anyhow::Result;
use tokio::sync::RwLock;

use super::graph::CausalGraph;
use super::indexer::CodebaseCausalIndexer;
use super::types::{
    BlastRadiusReport, CausalEdge, CausalNode, CausalNodeKind, PatchSimulationResult,
};

/// Cognitive World Model managing the causal architectural graph and pre-flight change simulations.
#[derive(Clone)]
pub struct WorldModel {
    graph: Arc<RwLock<CausalGraph>>,
    indexer: Arc<CodebaseCausalIndexer>,
}

impl Default for WorldModel {
    fn default() -> Self {
        Self::new()
    }
}

impl WorldModel {
    /// Create a new World Model initialized with core Strata architectural topology.
    pub fn new() -> Self {
        let mut graph = CausalGraph::new();
        let indexer = CodebaseCausalIndexer::new();
        indexer.seed_core_architecture_topology(&mut graph);

        Self {
            graph: Arc::new(RwLock::new(graph)),
            indexer: Arc::new(indexer),
        }
    }

    /// Create a World Model with a pre-configured Causal Graph.
    pub fn with_graph(graph: CausalGraph) -> Self {
        Self {
            graph: Arc::new(RwLock::new(graph)),
            indexer: Arc::new(CodebaseCausalIndexer::new()),
        }
    }

    /// Access inner Causal Graph with read lock.
    pub async fn read_graph(&self) -> tokio::sync::RwLockReadGuard<'_, CausalGraph> {
        self.graph.read().await
    }

    /// Scan and index a workspace directory into the causal world model.
    pub async fn index_workspace(&self, root_dir: &Path) -> Result<usize> {
        let mut graph = self.graph.write().await;
        let count = self.indexer.index_workspace(root_dir, &mut graph)?;
        Ok(count)
    }

    /// Register a dynamic architectural invariant into the causal graph.
    pub async fn register_invariant(
        &self,
        name: &str,
        description: &str,
        target_node_id: &str,
    ) -> Result<()> {
        let mut graph = self.graph.write().await;
        let node_id = format!("invariant:{}", name.to_lowercase().replace(' ', "_"));
        let node = CausalNode::new(node_id.clone(), name, CausalNodeKind::ContractInvariant)
            .with_metadata(serde_json::json!({ "description": description }));
        graph.add_node(node);

        if graph.get_node(target_node_id).is_some() {
            graph.add_edge(
                &node_id,
                target_node_id,
                CausalEdge::enforces_contract(1.0).with_description(description),
            )?;
        }
        Ok(())
    }

    /// Register a known failure pattern / anti-pattern barrier into the causal graph.
    pub async fn register_anti_pattern(
        &self,
        signature: &str,
        pattern_name: &str,
        mitigation: &str,
        target_node_id: &str,
    ) -> Result<()> {
        let mut graph = self.graph.write().await;
        let node_id = format!("antipattern:{signature}");
        let node = CausalNode::new(node_id.clone(), pattern_name, CausalNodeKind::ContractInvariant)
            .with_metadata(serde_json::json!({ "mitigation": mitigation }));
        graph.add_node(node);

        if graph.get_node(target_node_id).is_some() {
            graph.add_edge(
                &node_id,
                target_node_id,
                CausalEdge::enforces_contract(1.0).with_description(mitigation),
            )?;
        }
        Ok(())
    }

    /// Predict the blast radius and downstream ripple effects for a given target entity or file path.
    pub async fn predict_impact(&self, target_query: &str, depth: usize) -> Result<BlastRadiusReport> {
        let graph = self.graph.read().await;
        let effective_depth = if depth == 0 { 3 } else { depth };
        Ok(graph.compute_blast_radius(target_query, effective_depth))
    }

    /// Generate an ASCII dependency and impact tree for terminal display.
    pub async fn to_ascii_tree(&self, target_query: &str, depth: usize) -> Result<String> {
        let graph = self.graph.read().await;
        let effective_depth = if depth == 0 { 3 } else { depth };
        Ok(graph.to_ascii_tree(target_query, effective_depth))
    }

    /// Pre-flight simulate a set of proposed code changes across multiple files/modules.
    pub async fn simulate_patch(&self, modified_targets: &[String]) -> Result<PatchSimulationResult> {
        let graph = self.graph.read().await;

        let mut reports = Vec::new();
        let mut all_triggered_anti_patterns = Vec::new();
        let mut total_impacted_count = 0;
        let mut highest_risk: f32 = 0.0;
        let mut total_breaking = 0;

        for target in modified_targets {
            let report = graph.compute_blast_radius(target, 3);
            if report.overall_risk_score > highest_risk {
                highest_risk = report.overall_risk_score;
            }
            total_impacted_count += report.direct_impacts.len() + report.transitive_impacts.len();
            total_breaking += report.direct_impacts.iter().filter(|n| n.is_breaking_risk).count();
            total_breaking += report.transitive_impacts.iter().filter(|n| n.is_breaking_risk).count();

            for ap in &report.triggered_anti_patterns {
                if !all_triggered_anti_patterns.contains(ap) {
                    all_triggered_anti_patterns.push(ap.clone());
                }
            }
            for inv in &report.triggered_invariants {
                if !all_triggered_anti_patterns.contains(inv) {
                    all_triggered_anti_patterns.push(inv.clone());
                }
            }

            reports.push(report);
        }

        let safe_to_apply = highest_risk < 0.75 && all_triggered_anti_patterns.is_empty();

        Ok(PatchSimulationResult {
            modified_targets: modified_targets.to_vec(),
            total_impacted_nodes: total_impacted_count,
            highest_risk_score: highest_risk,
            breaking_risks_count: total_breaking,
            triggered_anti_patterns: all_triggered_anti_patterns,
            safe_to_apply,
            blast_reports: reports,
        })
    }
}
