use anyhow::{bail, Result};
use petgraph::algo::{is_cyclic_directed, toposort};
use petgraph::graph::{DiGraph, NodeIndex};
use petgraph::visit::EdgeRef;
use petgraph::Direction;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use super::types::{ExecutionWave, GoalEdge, GoalEdgeKind, GoalNode, GoalNodeKind, GoalStatus};

/// Serializable representation of a directed goal dependency edge.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SerializedGoalEdge {
    pub from: String,
    pub to: String,
    pub edge: GoalEdge,
}

/// Serializable export of the complete Goal DAG.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GoalDagExport {
    pub nodes: Vec<GoalNode>,
    pub edges: Vec<SerializedGoalEdge>,
}

/// Directed Acyclic Graph (DAG) of hierarchical goals, execution waves, and verification gates.
#[derive(Debug, Clone)]
pub struct GoalDag {
    graph: DiGraph<GoalNode, GoalEdge>,
    node_indices: HashMap<String, NodeIndex>,
    index_to_id: HashMap<NodeIndex, String>,
}

impl Default for GoalDag {
    fn default() -> Self {
        Self::new()
    }
}

impl Serialize for GoalDag {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let export = self.export();
        export.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for GoalDag {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let export = GoalDagExport::deserialize(deserializer)?;
        GoalDag::from_export(export).map_err(serde::de::Error::custom)
    }
}

impl GoalDag {
    pub fn new() -> Self {
        Self {
            graph: DiGraph::new(),
            node_indices: HashMap::new(),
            index_to_id: HashMap::new(),
        }
    }

    /// Add or update a node in the Goal DAG.
    pub fn add_node(&mut self, node: GoalNode) -> NodeIndex {
        let id = node.id.clone();
        if let Some(&idx) = self.node_indices.get(&id) {
            self.graph[idx] = node;
            idx
        } else {
            let idx = self.graph.add_node(node);
            self.node_indices.insert(id.clone(), idx);
            self.index_to_id.insert(idx, id);
            idx
        }
    }

    /// Add a directed dependency edge: `from_id -> to_id`.
    ///
    /// Semantics: `from_id` is a prerequisite of `to_id` (i.e. `from_id` executes BEFORE `to_id`).
    pub fn add_edge(&mut self, from_id: &str, to_id: &str, edge: GoalEdge) -> Result<()> {
        let from_idx = match self.node_indices.get(from_id) {
            Some(&i) => i,
            None => bail!("Source node not found in Goal DAG: '{from_id}'"),
        };
        let to_idx = match self.node_indices.get(to_id) {
            Some(&i) => i,
            None => bail!("Target node not found in Goal DAG: '{to_id}'"),
        };

        // Update existing edge if already present or add new
        if let Some(edge_idx) = self.graph.find_edge(from_idx, to_idx) {
            self.graph[edge_idx] = edge;
        } else {
            self.graph.add_edge(from_idx, to_idx, edge);
        }

        Ok(())
    }

    /// Helper to add a prerequisite execution dependency: `dependent` depends on `prerequisite`.
    /// Adds directed edge `prerequisite -> dependent`.
    pub fn add_dependency(&mut self, dependent: &str, prerequisite: &str) -> Result<()> {
        self.add_edge(prerequisite, dependent, GoalEdge::depends_on())
    }

    /// Check if a node ID exists in the DAG.
    pub fn contains_node(&self, id: &str) -> bool {
        self.node_indices.contains_key(id)
    }

    /// Retrieve a reference to a goal node by ID.
    pub fn get_node(&self, id: &str) -> Option<&GoalNode> {
        self.node_indices
            .get(id)
            .and_then(|&idx| self.graph.node_weight(idx))
    }

    /// Retrieve a mutable reference to a goal node by ID.
    pub fn get_node_mut(&mut self, id: &str) -> Option<&mut GoalNode> {
        let idx = *self.node_indices.get(id)?;
        self.graph.node_weight_mut(idx)
    }

    /// Update a node's lifecycle status.
    pub fn update_node_status(&mut self, id: &str, status: GoalStatus) -> Result<()> {
        let node = self
            .get_node_mut(id)
            .ok_or_else(|| anyhow::anyhow!("Goal node not found: '{id}'"))?;
        node.status = status;
        Ok(())
    }

    /// Total number of goal nodes in the DAG.
    pub fn node_count(&self) -> usize {
        self.graph.node_count()
    }

    /// Total number of dependency edges in the DAG.
    pub fn edge_count(&self) -> usize {
        self.graph.edge_count()
    }

    /// Retrieve all goal nodes in the DAG.
    pub fn all_nodes(&self) -> Vec<&GoalNode> {
        self.graph.node_weights().collect()
    }

    /// Retrieve all goal nodes mutably.
    pub fn all_nodes_mut(&mut self) -> Vec<&mut GoalNode> {
        self.graph.node_weights_mut().collect()
    }

    /// Retrieve all edges with their source and target node IDs.
    pub fn all_edges(&self) -> Vec<(String, String, GoalEdge)> {
        let mut list = Vec::new();
        for edge_ref in self.graph.edge_references() {
            let source_idx = edge_ref.source();
            let target_idx = edge_ref.target();
            if let (Some(source_id), Some(target_id)) = (
                self.index_to_id.get(&source_idx),
                self.index_to_id.get(&target_idx),
            ) {
                list.push((
                    source_id.clone(),
                    target_id.clone(),
                    edge_ref.weight().clone(),
                ));
            }
        }
        list
    }

    /// Returns the list of prerequisite node IDs that must complete before `node_id` can execute.
    pub fn get_prerequisites(&self, node_id: &str) -> Vec<String> {
        let mut prereqs = Vec::new();
        if let Some(&idx) = self.node_indices.get(node_id) {
            for neighbor_idx in self.graph.neighbors_directed(idx, Direction::Incoming) {
                if let Some(id) = self.index_to_id.get(&neighbor_idx) {
                    prereqs.push(id.clone());
                }
            }
        }
        prereqs.sort();
        prereqs
    }

    /// Returns the list of dependent node IDs that depend on `node_id` completing.
    pub fn get_dependents(&self, node_id: &str) -> Vec<String> {
        let mut dependents = Vec::new();
        if let Some(&idx) = self.node_indices.get(node_id) {
            for neighbor_idx in self.graph.neighbors_directed(idx, Direction::Outgoing) {
                if let Some(id) = self.index_to_id.get(&neighbor_idx) {
                    dependents.push(id.clone());
                }
            }
        }
        dependents.sort();
        dependents
    }

    /// Check if the DAG contains any directed cycles.
    pub fn contains_cycle(&self) -> bool {
        is_cyclic_directed(&self.graph)
    }

    /// Validate the graph topology: ensuring no cycles and non-empty node count.
    pub fn validate(&self) -> Result<()> {
        if self.graph.node_count() == 0 {
            bail!("Goal DAG is empty (0 nodes)");
        }
        if self.contains_cycle() {
            bail!("Goal DAG contains cyclic dependencies, violating DAG invariant");
        }
        Ok(())
    }

    /// Computes parallel execution waves using topological in-degree and longest path layering.
    ///
    /// Wave 0 consists of all nodes with in-degree 0 (no prerequisites).
    /// Wave `k` consists of nodes whose deepest prerequisite is at Wave `k-1`.
    pub fn compute_waves(&self) -> Result<Vec<ExecutionWave>> {
        self.validate()?;

        let sorted_indices = toposort(&self.graph, None)
            .map_err(|_| anyhow::anyhow!("Failed to compute topological sort for Goal DAG"))?;

        let mut node_wave_levels: HashMap<NodeIndex, usize> = HashMap::new();

        for &idx in &sorted_indices {
            let mut max_incoming_wave = None;
            for incoming_idx in self.graph.neighbors_directed(idx, Direction::Incoming) {
                let incoming_level = node_wave_levels.get(&incoming_idx).copied().unwrap_or(0);
                max_incoming_wave = Some(match max_incoming_wave {
                    Some(m) => std::cmp::max(m, incoming_level + 1),
                    None => incoming_level + 1,
                });
            }
            let wave_level = max_incoming_wave.unwrap_or(0);
            node_wave_levels.insert(idx, wave_level);
        }

        let max_wave = node_wave_levels.values().copied().max().unwrap_or(0);
        let mut waves: Vec<ExecutionWave> = (0..=max_wave)
            .map(|i| ExecutionWave::new(i, Vec::new()))
            .collect();

        for (&idx, &level) in &node_wave_levels {
            if let Some(id) = self.index_to_id.get(&idx) {
                waves[level].node_ids.push(id.clone());
            }
        }

        // Deterministically sort node IDs in each wave
        for wave in &mut waves {
            wave.node_ids.sort();
        }

        // Filter out empty waves if any
        waves.retain(|w| !w.node_ids.is_empty());

        // Re-index wave numbers
        for (i, wave) in waves.iter_mut().enumerate() {
            wave.wave_index = i;
        }

        Ok(waves)
    }

    /// Dynamic replanning patch: replaces a failed node with a replacement node preserving edges.
    pub fn patch_replace_node(&mut self, old_id: &str, new_node: GoalNode) -> Result<()> {
        let old_idx = match self.node_indices.get(old_id) {
            Some(&i) => i,
            None => bail!("Cannot replace node: '{old_id}' not found"),
        };

        let new_id = new_node.id.clone();
        let new_idx = self.add_node(new_node);

        // Copy incoming edges and remove old incoming edges
        let incoming: Vec<(NodeIndex, GoalEdge)> = self
            .graph
            .neighbors_directed(old_idx, Direction::Incoming)
            .filter_map(|pred_idx| {
                self.graph
                    .find_edge(pred_idx, old_idx)
                    .and_then(|e_idx| self.graph.edge_weight(e_idx).cloned())
                    .map(|edge| (pred_idx, edge))
            })
            .collect();

        for (pred_idx, edge) in incoming {
            self.graph.add_edge(pred_idx, new_idx, edge);
            if let Some(e_idx) = self.graph.find_edge(pred_idx, old_idx) {
                self.graph.remove_edge(e_idx);
            }
        }

        // Copy outgoing edges and remove old outgoing edges
        let outgoing: Vec<(NodeIndex, GoalEdge)> = self
            .graph
            .neighbors_directed(old_idx, Direction::Outgoing)
            .filter_map(|succ_idx| {
                self.graph
                    .find_edge(old_idx, succ_idx)
                    .and_then(|e_idx| self.graph.edge_weight(e_idx).cloned())
                    .map(|edge| (succ_idx, edge))
            })
            .collect();

        for (succ_idx, edge) in outgoing {
            self.graph.add_edge(new_idx, succ_idx, edge);
            if let Some(e_idx) = self.graph.find_edge(old_idx, succ_idx) {
                self.graph.remove_edge(e_idx);
            }
        }

        // Mark old node skipped
        if let Some(old) = self.get_node_mut(old_id) {
            old.mark_skipped(format!("Replaced by new node '{new_id}'"));
        }

        Ok(())
    }

    /// Dynamic replanning patch: injects mitigation nodes between failed node and its downstream dependents.
    pub fn patch_inject_mitigation(
        &mut self,
        failed_id: &str,
        mitigation_nodes: Vec<GoalNode>,
        custom_edges: Vec<(String, String, GoalEdgeKind)>,
    ) -> Result<()> {
        if mitigation_nodes.is_empty() {
            return Ok(());
        }

        let failed_idx = match self.node_indices.get(failed_id) {
            Some(&i) => i,
            None => bail!("Cannot inject mitigation: failed node '{failed_id}' not found"),
        };

        // Capture incoming predecessors of failed node
        let incoming_preds: Vec<(NodeIndex, GoalEdge)> = self
            .graph
            .neighbors_directed(failed_idx, Direction::Incoming)
            .filter_map(|pred_idx| {
                self.graph
                    .find_edge(pred_idx, failed_idx)
                    .and_then(|e_idx| self.graph.edge_weight(e_idx).cloned())
                    .map(|edge| (pred_idx, edge))
            })
            .collect();

        // Capture all existing outgoing dependents of the failed node and remove old edges
        let outgoing_edges: Vec<(NodeIndex, GoalEdge)> = self
            .graph
            .neighbors_directed(failed_idx, Direction::Outgoing)
            .filter_map(|succ_idx| {
                self.graph
                    .find_edge(failed_idx, succ_idx)
                    .and_then(|e_idx| self.graph.edge_weight(e_idx).cloned())
                    .map(|edge| (succ_idx, edge))
            })
            .collect();

        for (succ_idx, _) in &outgoing_edges {
            if let Some(e_idx) = self.graph.find_edge(failed_idx, *succ_idx) {
                self.graph.remove_edge(e_idx);
            }
        }

        // 1. Add all mitigation nodes
        let mut first_mitigation_id = None;
        let mut last_mitigation_id = None;

        for (i, node) in mitigation_nodes.into_iter().enumerate() {
            let id = node.id.clone();
            if i == 0 {
                first_mitigation_id = Some(id.clone());
            }
            last_mitigation_id = Some(id.clone());
            self.add_node(node);
        }

        // 2. Add custom edges between mitigation nodes
        for (from, to, kind) in custom_edges {
            let is_crit = matches!(kind, GoalEdgeKind::DependsOn | GoalEdgeKind::Verifies);
            let _ = self.add_edge(&from, &to, GoalEdge::new(kind, is_crit));
        }

        // 3. Connect incoming predecessors of failed node to first mitigation node
        if let Some(first_id) = first_mitigation_id {
            if let Some(&first_idx) = self.node_indices.get(&first_id) {
                for (pred_idx, edge) in incoming_preds {
                    self.graph.add_edge(pred_idx, first_idx, edge);
                }
            }
        }

        // 4. Connect last mitigation node -> downstream dependents
        if let Some(last_id) = last_mitigation_id {
            if let Some(&last_idx) = self.node_indices.get(&last_id) {
                for (dep_idx, edge) in outgoing_edges {
                    self.graph.add_edge(last_idx, dep_idx, edge);
                }
            }
        }

        // 5. Mark failed node as skipped (recovered via mitigation)
        if let Some(node) = self.get_node_mut(failed_id) {
            node.mark_skipped("Recovered via dynamic mitigation injection");
        }

        Ok(())
    }

    /// Dynamic replanning patch: bypasses a non-critical node, rewires incoming prerequisites directly to dependents.
    pub fn patch_bypass_node(&mut self, node_id: &str) -> Result<()> {
        let node_idx = match self.node_indices.get(node_id) {
            Some(&i) => i,
            None => bail!("Cannot bypass node: '{node_id}' not found"),
        };

        let incoming_preds: Vec<NodeIndex> = self
            .graph
            .neighbors_directed(node_idx, Direction::Incoming)
            .collect();
        let outgoing_succs: Vec<NodeIndex> = self
            .graph
            .neighbors_directed(node_idx, Direction::Outgoing)
            .collect();

        // Connect all predecessors directly to all successors
        for &pred_idx in &incoming_preds {
            for &succ_idx in &outgoing_succs {
                if self.graph.find_edge(pred_idx, succ_idx).is_none() {
                    self.graph
                        .add_edge(pred_idx, succ_idx, GoalEdge::depends_on());
                }
            }
        }

        // Remove old outgoing edges from bypassed node
        for &succ_idx in &outgoing_succs {
            if let Some(e_idx) = self.graph.find_edge(node_idx, succ_idx) {
                self.graph.remove_edge(e_idx);
            }
        }

        if let Some(node) = self.get_node_mut(node_id) {
            node.mark_skipped("Bypassed via dynamic recovery");
        }

        Ok(())
    }

    /// Export the DAG as a serializable struct.
    pub fn export(&self) -> GoalDagExport {
        let nodes: Vec<GoalNode> = self.graph.node_weights().cloned().collect();
        let mut edges = Vec::new();
        for edge_ref in self.graph.edge_references() {
            if let (Some(from), Some(to)) = (
                self.index_to_id.get(&edge_ref.source()),
                self.index_to_id.get(&edge_ref.target()),
            ) {
                edges.push(SerializedGoalEdge {
                    from: from.clone(),
                    to: to.clone(),
                    edge: edge_ref.weight().clone(),
                });
            }
        }
        GoalDagExport { nodes, edges }
    }

    /// Import a DAG from its exported struct.
    pub fn from_export(export: GoalDagExport) -> Result<Self> {
        let mut dag = Self::new();
        for node in export.nodes {
            dag.add_node(node);
        }
        for edge_item in export.edges {
            dag.add_edge(&edge_item.from, &edge_item.to, edge_item.edge)?;
        }
        Ok(dag)
    }

    /// Render a rich ASCII diagram of the execution waves and goal hierarchy.
    pub fn to_ascii_tree(&self) -> String {
        let waves = match self.compute_waves() {
            Ok(w) => w,
            Err(_) => return "⚠️ [Invalid or Cyclic Goal DAG - Cannot Render Waves]".to_string(),
        };

        let mut out = String::new();
        out.push_str(
            "┌────────────────────────────────────────────────────────────────────────┐\n",
        );
        out.push_str(
            "│  📋 STRATA HIERARCHICAL GOAL DAG EXECUTION PLAN                         │\n",
        );
        out.push_str(
            "└────────────────────────────────────────────────────────────────────────┘\n",
        );

        for wave in &waves {
            let parallel_desc = if wave.node_ids.len() > 1 {
                format!("Parallel Execution: {} goals", wave.node_ids.len())
            } else {
                "Sequential Goal".to_string()
            };

            out.push_str(&format!(
                "\n🌊 WAVE {} (\x1b[1;36m{}\x1b[0m)\n",
                wave.wave_index, parallel_desc
            ));

            for (i, node_id) in wave.node_ids.iter().enumerate() {
                let is_last = i == wave.node_ids.len() - 1;
                let branch = if is_last { "└──" } else { "├──" };

                if let Some(node) = self.get_node(node_id) {
                    let kind_badge = match node.kind {
                        GoalNodeKind::Root => "\x1b[1;35m[Root]\x1b[0m",
                        GoalNodeKind::Phase => "\x1b[1;34m[Phase]\x1b[0m",
                        GoalNodeKind::Task => "\x1b[1;32m[Task]\x1b[0m",
                        GoalNodeKind::Verification => "\x1b[1;33m[Verification]\x1b[0m",
                        GoalNodeKind::Rollback => "\x1b[1;31m[Rollback]\x1b[0m",
                    };

                    let status_badge = match node.status {
                        GoalStatus::Pending => "\x1b[90m[PENDING]\x1b[0m",
                        GoalStatus::Running => "\x1b[1;33m[RUNNING]\x1b[0m",
                        GoalStatus::Completed => "\x1b[1;32m[COMPLETED]\x1b[0m",
                        GoalStatus::Failed => "\x1b[1;31m[FAILED]\x1b[0m",
                        GoalStatus::Skipped => "\x1b[90m[SKIPPED]\x1b[0m",
                        GoalStatus::Blocked => "\x1b[31m[BLOCKED]\x1b[0m",
                    };

                    let prereqs = self.get_prerequisites(node_id);
                    let prereqs_str = if prereqs.is_empty() {
                        String::new()
                    } else {
                        format!(" \x1b[90m(after: {})\x1b[0m", prereqs.join(", "))
                    };

                    out.push_str(&format!(
                        "   {} {} {} \x1b[1m{}\x1b[0m: {}{}\n",
                        branch, kind_badge, status_badge, node.id, node.title, prereqs_str
                    ));
                }
            }
        }

        out
    }
}
