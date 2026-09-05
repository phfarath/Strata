use anyhow::{bail, Result};
use petgraph::graph::{DiGraph, NodeIndex};
use petgraph::Direction;
use std::collections::{HashMap, HashSet, VecDeque};

use super::types::{
    BlastRadiusReport, CausalEdge, CausalEdgeKind, CausalNode, CausalNodeKind, ImpactedNode,
};

/// Directed Causal Graph of Code Architecture, Data Flows, and Invariant Contracts.
#[derive(Debug, Clone)]
pub struct CausalGraph {
    graph: DiGraph<CausalNode, CausalEdge>,
    node_indices: HashMap<String, NodeIndex>,
    path_to_id: HashMap<String, String>,
}

impl Default for CausalGraph {
    fn default() -> Self {
        Self::new()
    }
}

impl CausalGraph {
    pub fn new() -> Self {
        Self {
            graph: DiGraph::new(),
            node_indices: HashMap::new(),
            path_to_id: HashMap::new(),
        }
    }

    /// Add or update a node in the causal graph.
    pub fn add_node(&mut self, node: CausalNode) -> NodeIndex {
        if let Some(&idx) = self.node_indices.get(&node.id) {
            // Update node in-place
            if let Some(path) = &node.path {
                self.path_to_id.insert(path.clone(), node.id.clone());
            }
            self.graph[idx] = node;
            idx
        } else {
            let id = node.id.clone();
            if let Some(path) = &node.path {
                self.path_to_id.insert(path.clone(), id.clone());
            }
            let idx = self.graph.add_node(node);
            self.node_indices.insert(id, idx);
            idx
        }
    }

    /// Add a directed dependency edge: `from_id -> to_id` with causal relationship semantics.
    ///
    /// Semantics: `from_id` depends on / calls / imports `to_id`.
    /// Therefore, if `to_id` changes, `from_id` is in its blast radius.
    pub fn add_edge(&mut self, from_id: &str, to_id: &str, edge: CausalEdge) -> Result<()> {
        let from_idx = match self.node_indices.get(from_id) {
            Some(&i) => i,
            None => bail!("Source node not found in causal graph: '{from_id}'"),
        };
        let to_idx = match self.node_indices.get(to_id) {
            Some(&i) => i,
            None => bail!("Target node not found in causal graph: '{to_id}'"),
        };

        // Avoid adding duplicate identical edges
        let existing = self.graph.find_edge(from_idx, to_idx);
        if let Some(edge_idx) = existing {
            self.graph[edge_idx] = edge;
        } else {
            self.graph.add_edge(from_idx, to_idx, edge);
        }

        Ok(())
    }

    /// Retrieve a reference to a node by ID.
    pub fn get_node(&self, id: &str) -> Option<&CausalNode> {
        self.node_indices
            .get(id)
            .and_then(|&idx| self.graph.node_weight(idx))
    }

    /// Lookup a node ID by its file path.
    pub fn get_node_by_path(&self, path: &str) -> Option<&CausalNode> {
        self.path_to_id.get(path).and_then(|id| self.get_node(id))
    }

    /// Number of nodes registered in the graph.
    pub fn node_count(&self) -> usize {
        self.graph.node_count()
    }

    /// Number of dependency edges registered in the graph.
    pub fn edge_count(&self) -> usize {
        self.graph.edge_count()
    }

    /// List all nodes in the graph.
    pub fn all_nodes(&self) -> Vec<&CausalNode> {
        self.graph.node_weights().collect()
    }

    /// Incoming dependencies: nodes that depend on / call / write to this node.
    pub fn incoming_dependencies(&self, id: &str) -> Vec<(&CausalNode, &CausalEdge)> {
        let Some(&idx) = self.node_indices.get(id) else {
            return Vec::new();
        };

        let mut results = Vec::new();
        let mut walker = self
            .graph
            .neighbors_directed(idx, Direction::Incoming)
            .detach();
        while let Some((edge_idx, neighbor_idx)) = walker.next(&self.graph) {
            if let (Some(node), Some(edge)) = (
                self.graph.node_weight(neighbor_idx),
                self.graph.edge_weight(edge_idx),
            ) {
                results.push((node, edge));
            }
        }
        results
    }

    /// Outgoing dependencies: nodes that this node depends on / calls.
    pub fn outgoing_dependencies(&self, id: &str) -> Vec<(&CausalNode, &CausalEdge)> {
        let Some(&idx) = self.node_indices.get(id) else {
            return Vec::new();
        };

        let mut results = Vec::new();
        let mut walker = self
            .graph
            .neighbors_directed(idx, Direction::Outgoing)
            .detach();
        while let Some((edge_idx, neighbor_idx)) = walker.next(&self.graph) {
            if let (Some(node), Some(edge)) = (
                self.graph.node_weight(neighbor_idx),
                self.graph.edge_weight(edge_idx),
            ) {
                results.push((node, edge));
            }
        }
        results
    }

    /// Compute the Blast Radius and Risk Score for a target node or file path.
    ///
    /// The algorithm performs a breadth-first search along **incoming** dependency edges
    /// (the consumers and callers that will experience ripple effects if `target_id` changes),
    /// propagating cumulative coupling weights, detecting breaking change risks, and
    /// finding relevant contract invariants.
    pub fn compute_blast_radius(&self, target_query: &str, max_depth: usize) -> BlastRadiusReport {
        // Resolve target ID from direct ID or path
        let target_id = if self.node_indices.contains_key(target_query) {
            target_query.to_string()
        } else if let Some(id) = self.path_to_id.get(target_query) {
            id.clone()
        } else {
            // Check substring match on IDs or names
            let matched = self.graph.node_weights().find(|n| {
                n.id.contains(target_query)
                    || n.name.contains(target_query)
                    || n.path.as_deref().unwrap_or("").contains(target_query)
            });
            match matched {
                Some(n) => n.id.clone(),
                None => {
                    return BlastRadiusReport {
                        target_id: target_query.to_string(),
                        target_name: target_query.to_string(),
                        max_depth,
                        total_nodes_scanned: self.node_count(),
                        direct_impacts: Vec::new(),
                        transitive_impacts: Vec::new(),
                        triggered_anti_patterns: Vec::new(),
                        triggered_invariants: Vec::new(),
                        overall_risk_score: 0.0,
                        recommendations: vec![format!(
                            "Node '{target_query}' not found in codebase causal graph."
                        )],
                    };
                }
            }
        };

        let target_idx = self.node_indices[&target_id];
        let target_node = &self.graph[target_idx];
        let target_name = target_node.name.clone();

        let mut direct_impacts = Vec::new();
        let mut transitive_impacts = Vec::new();
        let triggered_anti_patterns = Vec::new();
        let mut triggered_invariants = Vec::new();

        // Queue item: (NodeIndex, distance, cumulative_weight, causal_path, edge_kinds, is_breaking)
        type CausalQueueItem = (
            NodeIndex,
            usize,
            f32,
            Vec<String>,
            Vec<CausalEdgeKind>,
            bool,
        );
        let mut queue: VecDeque<CausalQueueItem> = VecDeque::new();
        let mut visited: HashSet<NodeIndex> = HashSet::new();

        visited.insert(target_idx);

        // Also check if any Invariant or Anti-Pattern directly attaches to the target
        for (neighbor, edge) in self.incoming_dependencies(&target_id) {
            if neighbor.kind == CausalNodeKind::ContractInvariant {
                triggered_invariants.push(format!(
                    "{}: {}",
                    neighbor.name,
                    edge.description.as_deref().unwrap_or("Enforces contract")
                ));
            }
        }

        // Initialize queue with incoming neighbors (direct dependents)
        let mut walker = self
            .graph
            .neighbors_directed(target_idx, Direction::Incoming)
            .detach();
        while let Some((edge_idx, neighbor_idx)) = walker.next(&self.graph) {
            if visited.insert(neighbor_idx) {
                let edge = &self.graph[edge_idx];
                let neighbor_node = &self.graph[neighbor_idx];

                if neighbor_node.kind == CausalNodeKind::ContractInvariant {
                    triggered_invariants.push(format!(
                        "{}: {}",
                        neighbor_node.name,
                        edge.description.as_deref().unwrap_or("Contract invariant")
                    ));
                }

                queue.push_back((
                    neighbor_idx,
                    1,
                    edge.weight,
                    vec![target_id.clone(), neighbor_node.id.clone()],
                    vec![edge.kind],
                    edge.is_breaking_if_changed,
                ));
            }
        }

        while let Some((curr_idx, dist, weight, path, edge_kinds, is_breaking)) = queue.pop_front()
        {
            let curr_node = &self.graph[curr_idx];

            let item = ImpactedNode {
                node_id: curr_node.id.clone(),
                name: curr_node.name.clone(),
                kind: curr_node.kind,
                path: curr_node.path.clone(),
                distance: dist,
                cumulative_weight: weight,
                is_breaking_risk: is_breaking,
                causal_path: path.clone(),
                edge_kinds: edge_kinds.clone(),
            };

            if dist == 1 {
                direct_impacts.push(item);
            } else {
                transitive_impacts.push(item);
            }

            // If we can traverse deeper, explore incoming dependencies of current node
            if dist < max_depth {
                let mut next_walker = self
                    .graph
                    .neighbors_directed(curr_idx, Direction::Incoming)
                    .detach();
                while let Some((next_edge_idx, next_neighbor_idx)) = next_walker.next(&self.graph) {
                    if visited.insert(next_neighbor_idx) {
                        let next_edge = &self.graph[next_edge_idx];
                        let next_node = &self.graph[next_neighbor_idx];

                        if next_node.kind == CausalNodeKind::ContractInvariant {
                            triggered_invariants.push(format!(
                                "{}: {}",
                                next_node.name,
                                next_edge
                                    .description
                                    .as_deref()
                                    .unwrap_or("Contract invariant")
                            ));
                        }

                        let next_weight = weight * next_edge.weight;
                        let next_breaking = is_breaking || next_edge.is_breaking_if_changed;

                        let mut next_path = path.clone();
                        next_path.push(next_node.id.clone());

                        let mut next_edge_kinds = edge_kinds.clone();
                        next_edge_kinds.push(next_edge.kind);

                        queue.push_back((
                            next_neighbor_idx,
                            dist + 1,
                            next_weight,
                            next_path,
                            next_edge_kinds,
                            next_breaking,
                        ));
                    }
                }
            }
        }

        // Calculate overall risk score [0.0, 1.0]
        let direct_weight_sum: f32 = direct_impacts.iter().map(|n| n.cumulative_weight).sum();
        let transitive_weight_sum: f32 = transitive_impacts
            .iter()
            .map(|n| n.cumulative_weight * 0.5)
            .sum();
        let breaking_multiplier = if direct_impacts
            .iter()
            .chain(&transitive_impacts)
            .any(|n| n.is_breaking_risk)
        {
            1.4
        } else {
            1.0
        };
        let invariant_penalty = if !triggered_invariants.is_empty() {
            0.25
        } else {
            0.0
        };

        let raw_score = ((direct_weight_sum * 0.3 + transitive_weight_sum * 0.15)
            * breaking_multiplier)
            + invariant_penalty;
        let overall_risk_score = raw_score.clamp(0.05, 1.0);

        // Generate recommendations
        let mut recommendations = Vec::new();
        if direct_impacts.is_empty() && transitive_impacts.is_empty() {
            recommendations.push(
                "Isolated node: modifying this component has zero upstream consumers.".to_string(),
            );
        } else {
            if direct_impacts.iter().any(|n| n.is_breaking_risk) {
                recommendations.push("⚠️ Breaking Change Warning: One or more direct dependents have strict contract dependencies.".to_string());
            }
            if !triggered_invariants.is_empty() {
                recommendations.push(format!(
                    "🛡️ Invariant Check: {} architectural invariants must be preserved.",
                    triggered_invariants.len()
                ));
            }
            if direct_impacts.len() > 4 {
                recommendations.push("High Fan-Out: Consider introducing an interface layer or staging changes through deprecation.".to_string());
            }
            recommendations.push(format!(
                "Run targeted test suites covering: {} direct dependent modules.",
                direct_impacts.len()
            ));
        }

        BlastRadiusReport {
            target_id,
            target_name,
            max_depth,
            total_nodes_scanned: self.node_count(),
            direct_impacts,
            transitive_impacts,
            triggered_anti_patterns,
            triggered_invariants,
            overall_risk_score,
            recommendations,
        }
    }

    /// Render ASCII dependency tree showing causal blast radius ripple effect.
    pub fn to_ascii_tree(&self, target_id: &str, max_depth: usize) -> String {
        let report = self.compute_blast_radius(target_id, max_depth);
        let mut out = String::new();

        out.push_str(&format!(
            "🎯 Target: {} [{}]\n",
            report.target_name, report.target_id
        ));
        out.push_str(&format!(
            "   Overall Blast Risk Score: {:.1}%\n",
            report.overall_risk_score * 100.0
        ));
        out.push_str("   Direct & Transitive Upstream Dependents:\n");

        if report.direct_impacts.is_empty() && report.transitive_impacts.is_empty() {
            out.push_str("   └── (No upstream dependents — isolated component)\n");
            return out;
        }

        for (i, d) in report.direct_impacts.iter().enumerate() {
            let is_last_direct =
                i == report.direct_impacts.len() - 1 && report.transitive_impacts.is_empty();
            let branch = if is_last_direct {
                "└──"
            } else {
                "├──"
            };
            let breaking_badge = if d.is_breaking_risk {
                " [BREAKING RISK]"
            } else {
                ""
            };

            out.push_str(&format!(
                "   {} (d=1) {} [{}] (coupling: {:.0}%){}\n",
                branch,
                d.name,
                d.kind,
                d.cumulative_weight * 100.0,
                breaking_badge
            ));
        }

        for (i, t) in report.transitive_impacts.iter().enumerate() {
            let is_last = i == report.transitive_impacts.len() - 1;
            let branch = if is_last { "└──" } else { "├──" };
            let breaking_badge = if t.is_breaking_risk {
                " [BREAKING RISK]"
            } else {
                ""
            };

            out.push_str(&format!(
                "   │   {} (d={}) {} [{}] (coupling: {:.0}%){}\n",
                branch,
                t.distance,
                t.name,
                t.kind,
                t.cumulative_weight * 100.0,
                breaking_badge
            ));
        }

        out
    }
}
