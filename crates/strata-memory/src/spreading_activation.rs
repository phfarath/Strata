use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};
use uuid::Uuid;

use crate::store::SqliteStore;
use strata_core::errors::StrataError;
use strata_core::state::Scope;

/// Node types in the associative Knowledge Graph.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum GraphNode {
    /// Persistent memory record (Episodic, Semantic, Procedural, NegativePattern)
    Memory(Uuid),
    /// Semantic fact from truth maintenance
    Fact(Uuid),
    /// Function, struct, or class symbol from AST or call graph
    Symbol(String),
    /// Source code file path
    File(String),
    /// Shared concept, tag, or error domain
    Concept(String),
}

impl GraphNode {
    pub fn memory(id: Uuid) -> Self {
        GraphNode::Memory(id)
    }

    pub fn fact(id: Uuid) -> Self {
        GraphNode::Fact(id)
    }

    pub fn symbol(s: impl Into<String>) -> Self {
        GraphNode::Symbol(s.into())
    }

    pub fn file(f: impl Into<String>) -> Self {
        GraphNode::File(f.into())
    }

    pub fn concept(c: impl Into<String>) -> Self {
        GraphNode::Concept(c.into().to_lowercase())
    }

    pub fn as_memory_id(&self) -> Option<Uuid> {
        match self {
            GraphNode::Memory(id) => Some(*id),
            _ => None,
        }
    }

    pub fn as_fact_id(&self) -> Option<Uuid> {
        match self {
            GraphNode::Fact(id) => Some(*id),
            _ => None,
        }
    }
}

/// Semantic relationship categories connecting nodes in the Knowledge Graph.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EdgeKind {
    /// Causal, justification, or truth dependency (fact_dependencies, evidence_ids)
    Supports,
    /// Memory or fact references a code symbol
    ReferencesSymbol,
    /// Memory or fact anchors to a file path
    MentionsFile,
    /// AST invocation between caller and callee
    Calls,
    /// Temporal co-occurrence in the same task/session
    CoOccurred,
    /// Shared semantic category or tag
    SharedTag,
}

impl EdgeKind {
    pub fn default_weight(&self) -> f32 {
        match self {
            EdgeKind::Supports => 0.95,
            EdgeKind::ReferencesSymbol => 0.85,
            EdgeKind::MentionsFile => 0.80,
            EdgeKind::Calls => 0.75,
            EdgeKind::CoOccurred => 0.60,
            EdgeKind::SharedTag => 0.50,
        }
    }
}

/// A weighted, typed edge connecting a source node to a target node.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GraphEdge {
    pub target: GraphNode,
    pub weight: f32,
    pub kind: EdgeKind,
}

/// Knowledge Graph storing associative connections between memories, facts, symbols, and files.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct KnowledgeGraph {
    adj: HashMap<GraphNode, Vec<GraphEdge>>,
    node_set: HashSet<GraphNode>,
    edge_count: usize,
}

impl KnowledgeGraph {
    pub fn new() -> Self {
        Self {
            adj: HashMap::new(),
            node_set: HashSet::new(),
            edge_count: 0,
        }
    }

    pub fn add_node(&mut self, node: GraphNode) {
        self.node_set.insert(node.clone());
        self.adj.entry(node).or_default();
    }

    pub fn has_node(&self, node: &GraphNode) -> bool {
        self.node_set.contains(node)
    }

    pub fn node_count(&self) -> usize {
        self.node_set.len()
    }

    pub fn edge_count(&self) -> usize {
        self.edge_count
    }

    pub fn add_directed_edge(
        &mut self,
        from: GraphNode,
        to: GraphNode,
        weight: f32,
        kind: EdgeKind,
    ) {
        self.add_node(from.clone());
        self.add_node(to.clone());

        let edges = self.adj.entry(from).or_default();
        if let Some(existing) = edges.iter_mut().find(|e| e.target == to && e.kind == kind) {
            if weight > existing.weight {
                existing.weight = weight;
            }
        } else {
            edges.push(GraphEdge {
                target: to,
                weight,
                kind,
            });
            self.edge_count += 1;
        }
    }

    /// Add an associative bidirectional connection (e.g., shared tag, co-occurrence, or caller/callee link)
    pub fn add_bidirectional_edge(
        &mut self,
        a: GraphNode,
        b: GraphNode,
        weight: f32,
        kind: EdgeKind,
    ) {
        self.add_directed_edge(a.clone(), b.clone(), weight, kind);
        // Back-link has slightly attenuated energy diffusion (85% of forward weight)
        self.add_directed_edge(b, a, weight * 0.85, kind);
    }

    pub fn neighbors(&self, node: &GraphNode) -> &[GraphEdge] {
        self.adj.get(node).map(|v| v.as_slice()).unwrap_or(&[])
    }

    pub fn nodes(&self) -> impl Iterator<Item = &GraphNode> {
        self.node_set.iter()
    }

    /// Find nodes matching a query token (case-insensitive substring match for symbols, files, concepts).
    pub fn find_matching_nodes(&self, token: &str) -> Vec<GraphNode> {
        let token_lower = token.to_lowercase();
        let mut matches = Vec::new();

        for node in &self.node_set {
            match node {
                GraphNode::Symbol(s) if s.to_lowercase().contains(&token_lower) => {
                    matches.push(node.clone());
                }
                GraphNode::File(f) if f.to_lowercase().contains(&token_lower) => {
                    matches.push(node.clone());
                }
                GraphNode::Concept(c) if c.contains(&token_lower) => {
                    matches.push(node.clone());
                }
                _ => {}
            }
        }
        matches
    }
}

/// Configuration parameters for the Spreading Activation algorithm.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpreadingActivationConfig {
    /// Maximum number of diffusion hops across edges (default: 3)
    pub max_hops: usize,
    /// Energy decay factor per hop (lambda: 0.0 - 1.0, default: 0.65)
    pub decay_factor: f32,
    /// Minimum activation threshold to fire a node (theta: default: 0.05)
    pub activation_threshold: f32,
    /// Self-retention rate of current energy between iterations (alpha: default: 0.20)
    pub retention_rate: f32,
    /// Maximum number of actively radiating nodes to avoid quadratic blowup in large graphs
    pub max_active_nodes: usize,
}

impl Default for SpreadingActivationConfig {
    fn default() -> Self {
        Self {
            max_hops: 3,
            decay_factor: 0.65,
            activation_threshold: 0.05,
            retention_rate: 0.20,
            max_active_nodes: 500,
        }
    }
}

/// Deterministic Spreading Activation Engine (ACT-R & HippoRAG style).
#[derive(Debug, Clone)]
pub struct SpreadingActivationEngine {
    config: SpreadingActivationConfig,
}

impl Default for SpreadingActivationEngine {
    fn default() -> Self {
        Self::with_default_config()
    }
}

impl SpreadingActivationEngine {
    pub fn new(config: SpreadingActivationConfig) -> Self {
        Self { config }
    }

    pub fn with_default_config() -> Self {
        Self::new(SpreadingActivationConfig::default())
    }

    pub fn config(&self) -> &SpreadingActivationConfig {
        &self.config
    }

    /// Builds a connected Knowledge Graph from the persistent SQLite store.
    pub fn build_graph_from_store(
        &self,
        store: &SqliteStore,
        scope: Option<&Scope>,
    ) -> Result<KnowledgeGraph, StrataError> {
        let mut graph = KnowledgeGraph::new();

        // 1. Ingest memories in scope
        let memories = store.get_all_memories(scope, None, 1000)?;
        let mut session_groups: HashMap<String, Vec<Uuid>> = HashMap::new();
        let mut _tag_groups: HashMap<String, Vec<Uuid>> = HashMap::new();

        for mem in &memories {
            let mem_node = GraphNode::memory(mem.id);
            graph.add_node(mem_node.clone());

            // A. Link to evidence_ids (Supports)
            for &ev_id in &mem.evidence_ids {
                let ev_node = GraphNode::memory(ev_id);
                graph.add_directed_edge(
                    mem_node.clone(),
                    ev_node,
                    EdgeKind::Supports.default_weight(),
                    EdgeKind::Supports,
                );
            }

            // B. Extract concepts/tags
            for tag in &mem.tags {
                let concept_node = GraphNode::concept(tag);
                graph.add_bidirectional_edge(
                    mem_node.clone(),
                    concept_node,
                    EdgeKind::SharedTag.default_weight(),
                    EdgeKind::SharedTag,
                );
                _tag_groups
                    .entry(tag.to_lowercase())
                    .or_default()
                    .push(mem.id);
            }

            // C. Extract metadata links (files, symbols, sessions)
            if let Some(meta_obj) = mem.metadata.as_object() {
                if let Some(session_id) = meta_obj.get("session_id").and_then(|v| v.as_str()) {
                    session_groups
                        .entry(session_id.to_string())
                        .or_default()
                        .push(mem.id);
                }

                if let Some(file_paths) = meta_obj.get("file_paths").and_then(|v| v.as_array()) {
                    for fp in file_paths.iter().filter_map(|v| v.as_str()) {
                        let file_node = GraphNode::file(fp);
                        graph.add_bidirectional_edge(
                            mem_node.clone(),
                            file_node,
                            EdgeKind::MentionsFile.default_weight(),
                            EdgeKind::MentionsFile,
                        );
                    }
                }

                if let Some(symbols) = meta_obj.get("symbols").and_then(|v| v.as_array()) {
                    for sym in symbols.iter().filter_map(|v| v.as_str()) {
                        let sym_node = GraphNode::symbol(sym);
                        graph.add_bidirectional_edge(
                            mem_node.clone(),
                            sym_node,
                            EdgeKind::ReferencesSymbol.default_weight(),
                            EdgeKind::ReferencesSymbol,
                        );
                    }
                }
            }

            // Heuristic symbol & file extraction from content if not explicitly in metadata
            self.extract_inline_entities(&mem.content, &mem_node, &mut graph);
        }

        // Connect memories co-occurring in the same session (CoOccurred)
        for (_session_id, mem_ids) in session_groups {
            if mem_ids.len() > 1 && mem_ids.len() <= 20 {
                for i in 0..mem_ids.len() {
                    for j in (i + 1)..mem_ids.len() {
                        graph.add_bidirectional_edge(
                            GraphNode::memory(mem_ids[i]),
                            GraphNode::memory(mem_ids[j]),
                            EdgeKind::CoOccurred.default_weight(),
                            EdgeKind::CoOccurred,
                        );
                    }
                }
            }
        }

        // 2. Ingest Semantic Facts & fact_dependencies
        let facts = store
            .get_all_semantic_facts(None, None, 500)
            .unwrap_or_default();
        for fact in &facts {
            let fact_node = GraphNode::fact(fact.id);
            graph.add_node(fact_node.clone());

            // Link fact to its code anchor
            if let Some(anchor) = &fact.code_anchor {
                let file_node = GraphNode::file(&anchor.file_path);
                graph.add_bidirectional_edge(
                    fact_node.clone(),
                    file_node,
                    EdgeKind::MentionsFile.default_weight(),
                    EdgeKind::MentionsFile,
                );

                if !anchor.symbol_path.is_empty() {
                    let sym_node = GraphNode::symbol(&anchor.symbol_path);
                    graph.add_bidirectional_edge(
                        fact_node.clone(),
                        sym_node,
                        EdgeKind::ReferencesSymbol.default_weight(),
                        EdgeKind::ReferencesSymbol,
                    );
                }
            }

            // Link fact to its tags
            for tag in &fact.tags {
                let concept_node = GraphNode::concept(tag);
                graph.add_bidirectional_edge(
                    fact_node.clone(),
                    concept_node,
                    EdgeKind::SharedTag.default_weight(),
                    EdgeKind::SharedTag,
                );
            }
        }

        if let Ok(deps) = store.get_all_fact_dependencies() {
            for (dep_id, prereq_id, _dep_type) in deps {
                graph.add_directed_edge(
                    GraphNode::fact(dep_id),
                    GraphNode::fact(prereq_id),
                    EdgeKind::Supports.default_weight(),
                    EdgeKind::Supports,
                );
            }
        }

        // 3. Ingest Call Graph edges
        if let Ok(call_edges) = store.get_all_call_edges(500) {
            for edge in call_edges {
                let caller_node = GraphNode::symbol(&edge.caller_symbol);
                let callee_node = GraphNode::symbol(&edge.callee_symbol);
                let caller_file = GraphNode::file(&edge.caller_file);

                graph.add_directed_edge(
                    caller_node.clone(),
                    callee_node,
                    EdgeKind::Calls.default_weight(),
                    EdgeKind::Calls,
                );
                graph.add_bidirectional_edge(
                    caller_node,
                    caller_file,
                    EdgeKind::MentionsFile.default_weight(),
                    EdgeKind::MentionsFile,
                );
            }
        }

        Ok(graph)
    }

    /// Extract common code patterns (e.g. `src/foo.rs`, `struct Bar`, `fn baz`) as graph entities
    fn extract_inline_entities(
        &self,
        content: &str,
        mem_node: &GraphNode,
        graph: &mut KnowledgeGraph,
    ) {
        for word in content.split_whitespace() {
            let cleaned = word
                .trim_matches(|c: char| !c.is_alphanumeric() && c != '_' && c != '/' && c != '.');
            if cleaned.ends_with(".rs") || cleaned.ends_with(".ts") || cleaned.ends_with(".py") {
                let file_node = GraphNode::file(cleaned);
                graph.add_bidirectional_edge(
                    mem_node.clone(),
                    file_node,
                    EdgeKind::MentionsFile.default_weight() * 0.9,
                    EdgeKind::MentionsFile,
                );
            } else if cleaned.len() >= 4 && (cleaned.contains("::") || cleaned.contains('_')) {
                let sym_node = GraphNode::symbol(cleaned);
                graph.add_bidirectional_edge(
                    mem_node.clone(),
                    sym_node,
                    EdgeKind::ReferencesSymbol.default_weight() * 0.8,
                    EdgeKind::ReferencesSymbol,
                );
            }
        }
    }

    /// Propagates activation energy from seed nodes throughout the Knowledge Graph.
    ///
    /// Implements iterative discrete energy spreading:
    /// Delta A_t(v) = sum_{u in N(v)} A_{t-1}(u) * w(u, v) * lambda
    /// A_t(v) = min(1.0, A_{t-1}(v) * alpha + Delta A_t(v))
    pub fn propagate(
        &self,
        graph: &KnowledgeGraph,
        seeds: &[(GraphNode, f32)],
    ) -> HashMap<GraphNode, f32> {
        let mut current_activations: HashMap<GraphNode, f32> = HashMap::new();

        // 1. Seed initial activation
        for (node, energy) in seeds {
            if graph.has_node(node) {
                let clamped = energy.clamp(0.0, 1.0);
                let entry = current_activations.entry(node.clone()).or_insert(0.0);
                *entry = entry.max(clamped);
            }
        }

        if current_activations.is_empty() {
            return current_activations;
        }

        // 2. Iterative diffusion loop
        for _hop in 0..self.config.max_hops {
            let mut delta_activations: HashMap<GraphNode, f32> = HashMap::new();

            // Select active radiating nodes sorted by energy
            let mut active_nodes: Vec<(&GraphNode, f32)> = current_activations
                .iter()
                .filter(|(_, &a)| a >= self.config.activation_threshold)
                .map(|(n, &a)| (n, a))
                .collect();

            active_nodes.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
            active_nodes.truncate(self.config.max_active_nodes);

            for (node, energy) in active_nodes {
                let neighbors = graph.neighbors(node);
                if neighbors.is_empty() {
                    continue;
                }

                let transmit_energy = energy * self.config.decay_factor;

                for edge in neighbors {
                    let transferred = transmit_energy * edge.weight;
                    if transferred >= 0.005 {
                        *delta_activations.entry(edge.target.clone()).or_insert(0.0) += transferred;
                    }
                }
            }

            if delta_activations.is_empty() {
                break; // Energy depleted below threshold
            }

            // 3. Accumulate with retention and saturation capping
            let mut max_change = 0.0f32;
            for (node, delta) in delta_activations {
                let old_val = current_activations.get(&node).copied().unwrap_or(0.0);
                let new_val = (old_val * self.config.retention_rate + delta).min(1.0);
                let diff = (new_val - old_val).abs();
                if diff > max_change {
                    max_change = diff;
                }
                current_activations.insert(node, new_val);
            }

            // Early stopping if graph converged
            if max_change < 0.001 {
                break;
            }
        }

        current_activations
    }

    /// Extract ranked MemoryRecord IDs based on accumulated activation energy.
    pub fn extract_top_memories(
        &self,
        activations: &HashMap<GraphNode, f32>,
        limit: usize,
    ) -> Vec<(Uuid, f32)> {
        let mut memory_scores: Vec<(Uuid, f32)> = activations
            .iter()
            .filter_map(|(node, &score)| match node {
                GraphNode::Memory(id) => Some((*id, score)),
                _ => None,
            })
            .collect();

        memory_scores.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        memory_scores.truncate(limit);
        memory_scores
    }

    /// Find an explanatory associative path from a seed node to a target node.
    pub fn find_activation_path(
        &self,
        graph: &KnowledgeGraph,
        source: &GraphNode,
        target: &GraphNode,
        max_depth: usize,
    ) -> Option<Vec<(GraphNode, EdgeKind)>> {
        if source == target {
            return Some(vec![(source.clone(), EdgeKind::Supports)]);
        }

        let mut queue: VecDeque<(GraphNode, Vec<(GraphNode, EdgeKind)>)> = VecDeque::new();
        let mut visited: HashSet<GraphNode> = HashSet::new();

        queue.push_back((source.clone(), Vec::new()));
        visited.insert(source.clone());

        while let Some((current, path)) = queue.pop_front() {
            if path.len() >= max_depth {
                continue;
            }

            for edge in graph.neighbors(&current) {
                let mut new_path = path.clone();
                new_path.push((edge.target.clone(), edge.kind));

                if &edge.target == target {
                    return Some(new_path);
                }

                if visited.insert(edge.target.clone()) {
                    queue.push_back((edge.target.clone(), new_path));
                }
            }
        }

        None
    }

    /// Generate initial activation seeds from query keywords, files, symbols, and concepts.
    pub fn seed_from_query(&self, graph: &KnowledgeGraph, query: &str) -> Vec<(GraphNode, f32)> {
        let mut seeds: HashMap<GraphNode, f32> = HashMap::new();
        let words: Vec<&str> = query.split_whitespace().collect();

        for word in words {
            let cleaned = word
                .trim_matches(|c: char| !c.is_alphanumeric() && c != '_' && c != '/' && c != '.');
            if cleaned.is_empty() {
                continue;
            }

            let matching_nodes = graph.find_matching_nodes(cleaned);
            for node in matching_nodes {
                let weight = match node {
                    GraphNode::Symbol(_) => 1.0,
                    GraphNode::File(_) => 0.9,
                    GraphNode::Concept(_) => 0.7,
                    _ => 0.5,
                };
                let entry = seeds.entry(node).or_insert(0.0);
                *entry = entry.max(weight);
            }
        }

        seeds.into_iter().collect()
    }
}
