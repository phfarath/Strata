use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};
use strata_core::errors::StrataError;
use strata_core::state::{FailurePattern, MemoryRecord, MemoryType, Scope};
use uuid::Uuid;

use crate::call_graph::CallEdge;
use crate::compiler::{matches_path_glob, PathScopedExt};
use crate::embedding::{
    cosine_similarity, is_embedding_compatible_with_query, EmbeddingProvider,
};
use crate::spreading_activation::{
    GraphNode, SpreadingActivationConfig, SpreadingActivationEngine,
};
use crate::store::SqliteStore;

/// Computes the proximity decay factor for a given import/call graph hop distance:
/// - `0` hops (direct active file match): `1.5` (highest priority for active cursor file)
/// - `1` hop (direct 1-hop import/call neighbor): `1.0`
/// - `2` hops (transitive 2-hop import/call neighbor): `0.5`
/// - `>= 3` hops (distant or unconnected): `0.0`
pub fn import_distance_decay_factor(hops: usize) -> f32 {
    match hops {
        0 => 1.5,
        1 => 1.0,
        2 => 0.5,
        _ => 0.0,
    }
}

/// Configuration for the Active File & Cursor Proximity Biasing Engine.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProximityBiasConfig {
    /// Weight applied to the import distance decay factor when boosting memory scores (default: 0.35).
    pub proximity_weight: f32,
    /// Maximum import/call graph hops to traverse from active files (default: 2).
    pub max_import_hops: usize,
    /// Whether import/call edges are traversed bidirectionally (default: true).
    pub bidirectional_edges: bool,
}

impl Default for ProximityBiasConfig {
    fn default() -> Self {
        Self {
            proximity_weight: 0.35,
            max_import_hops: 2,
            bidirectional_edges: true,
        }
    }
}

/// Engine that biases memory retrieval and session digests toward the developer's active files
/// and their 1-hop / 2-hop import & call graph neighbors.
#[derive(Debug, Clone, Default)]
pub struct ProximityBiasEngine {
    config: ProximityBiasConfig,
    adjacency: HashMap<String, HashSet<String>>,
}

impl ProximityBiasEngine {
    pub fn new(config: ProximityBiasConfig) -> Self {
        Self {
            config,
            adjacency: HashMap::new(),
        }
    }

    pub fn with_default_config() -> Self {
        Self::new(ProximityBiasConfig::default())
    }

    pub fn config(&self) -> &ProximityBiasConfig {
        &self.config
    }

    /// Normalizes a file path by converting `\` to `/`, stripping leading `./` and `/`,
    /// and collapsing duplicate slashes.
    pub fn normalize_file_path(path: &str) -> String {
        let replaced = path.trim().replace('\\', "/");
        let stripped = replaced.trim_start_matches("./").trim_start_matches('/');
        let mut out = String::with_capacity(stripped.len());
        let mut prev_slash = false;
        for ch in stripped.chars() {
            if ch == '/' {
                if !prev_slash {
                    out.push(ch);
                }
                prev_slash = true;
            } else {
                out.push(ch);
                prev_slash = false;
            }
        }
        out
    }

    /// Registers an import or call dependency between `from_file` and `to_file`.
    pub fn add_import_edge(&mut self, from_file: &str, to_file: &str) {
        let from_norm = Self::normalize_file_path(from_file);
        let to_norm = Self::normalize_file_path(to_file);
        if from_norm.is_empty() || to_norm.is_empty() || from_norm == to_norm {
            return;
        }
        self.adjacency
            .entry(from_norm.clone())
            .or_default()
            .insert(to_norm.clone());
        if self.config.bidirectional_edges {
            self.adjacency.entry(to_norm).or_default().insert(from_norm);
        }
    }

    /// Builds a `ProximityBiasEngine` from a slice of `CallEdge` records.
    pub fn from_call_edges(edges: &[CallEdge], config: ProximityBiasConfig) -> Self {
        let mut engine = Self::new(config);
        for edge in edges {
            if let Some(ref target_file) = edge.callee_file_hint {
                engine.add_import_edge(&edge.caller_file, target_file);
            } else if looks_like_file_path(&edge.callee_symbol) {
                engine.add_import_edge(&edge.caller_file, &edge.callee_symbol);
            }
        }
        engine
    }

    /// Builds a `ProximityBiasEngine` from the persistent `SqliteStore` call graph.
    pub fn from_store(
        store: &SqliteStore,
        config: ProximityBiasConfig,
    ) -> Result<Self, StrataError> {
        let edges = store.get_all_call_edges(5000).unwrap_or_default();
        Ok(Self::from_call_edges(&edges, config))
    }

    /// Computes shortest hop distances (up to `config.max_import_hops`) from `active_files`
    /// across the file import/call graph using Breadth-First Search (BFS).
    pub fn compute_hop_distances(&self, active_files: &[String]) -> HashMap<String, usize> {
        let mut distances: HashMap<String, usize> = HashMap::new();
        let mut queue: VecDeque<(String, usize)> = VecDeque::new();

        for af in active_files {
            let norm = Self::normalize_file_path(af);
            if !norm.is_empty() && !distances.contains_key(&norm) {
                distances.insert(norm.clone(), 0);
                queue.push_back((norm, 0));
            }
        }

        while let Some((current, dist)) = queue.pop_front() {
            if dist >= self.config.max_import_hops {
                continue;
            }
            if let Some(neighbors) = self.adjacency.get(&current) {
                let mut sorted_neighbors: Vec<&String> = neighbors.iter().collect();
                sorted_neighbors.sort();
                for neighbor in sorted_neighbors {
                    if !distances.contains_key(neighbor) {
                        let next_dist = dist + 1;
                        distances.insert(neighbor.clone(), next_dist);
                        queue.push_back((neighbor.clone(), next_dist));
                    }
                }
            }
        }

        distances
    }

    /// Returns the decay factor for a given hop distance (salto 1 = 1.0, salto 2 = 0.5).
    pub fn decay_factor_for_hops(&self, hops: usize) -> f32 {
        if hops > self.config.max_import_hops {
            0.0
        } else {
            import_distance_decay_factor(hops)
        }
    }

    /// Extracts all normalized file paths associated with a `MemoryRecord`.
    pub fn extract_memory_files(record: &MemoryRecord) -> Vec<String> {
        let mut files = Vec::new();
        let mut seen = HashSet::new();

        let mut push_file = |raw: &str| {
            let norm = Self::normalize_file_path(raw);
            if !norm.is_empty() && seen.insert(norm.clone()) {
                files.push(norm);
            }
        };

        if let Some(meta) = record.metadata.as_object() {
            for key in ["file_path", "file", "source_file", "target_file"] {
                if let Some(fp) = meta.get(key).and_then(|v| v.as_str()) {
                    push_file(fp);
                }
            }
            if let Some(arr) = meta.get("file_paths").and_then(|v| v.as_array()) {
                for item in arr.iter().filter_map(|v| v.as_str()) {
                    push_file(item);
                }
            }
            if let Some(anchor) = meta.get("code_anchor").and_then(|v| v.as_object()) {
                if let Some(fp) = anchor.get("file_path").and_then(|v| v.as_str()) {
                    push_file(fp);
                }
            }
        }

        for tag in &record.tags {
            if let Some(fp) = tag.strip_prefix("file:") {
                push_file(fp);
            }
        }

        for token in extract_inline_file_paths(&record.content) {
            push_file(&token);
        }
        if let Some(ref summary) = record.summary {
            for token in extract_inline_file_paths(summary) {
                push_file(&token);
            }
        }

        files
    }

    /// Extracts all normalized file paths associated with a `FailurePattern`.
    pub fn extract_failure_files(failure: &FailurePattern) -> Vec<String> {
        let mut files = Vec::new();
        let mut seen = HashSet::new();

        let mut push_file = |raw: &str| {
            let norm = Self::normalize_file_path(raw);
            if !norm.is_empty() && seen.insert(norm.clone()) {
                files.push(norm);
            }
        };

        if let Some(meta) = failure.metadata.as_object() {
            for key in ["file_path", "file", "source_file", "target_file"] {
                if let Some(fp) = meta.get(key).and_then(|v| v.as_str()) {
                    push_file(fp);
                }
            }
            if let Some(ctx) = meta.get("context").and_then(|v| v.as_str()) {
                for token in extract_inline_file_paths(ctx) {
                    push_file(&token);
                }
            }
        }

        for token in extract_inline_file_paths(&failure.trigger_condition) {
            push_file(&token);
        }
        for token in extract_inline_file_paths(&failure.description) {
            push_file(&token);
        }

        files
    }

    /// Determines the minimum import hop distance between a `MemoryRecord` and the active file hop map.
    pub fn min_hop_distance_for_memory(
        &self,
        record: &MemoryRecord,
        hop_map: &HashMap<String, usize>,
    ) -> Option<usize> {
        if hop_map.is_empty() {
            return None;
        }

        let mut min_hops: Option<usize> = None;
        let mut update_min = |h: usize| {
            min_hops = Some(match min_hops {
                Some(curr) => curr.min(h),
                None => h,
            });
        };

        for file in Self::extract_memory_files(record) {
            if let Some(&h) = hop_map.get(&file) {
                update_min(h);
            } else {
                for (graph_file, &h) in hop_map {
                    if paths_match_suffix(graph_file, &file) {
                        update_min(h);
                    }
                }
            }
        }

        if let Some(glob) = record.path_glob() {
            let trimmed = glob.trim();
            if !trimmed.is_empty() && trimmed != "*" && trimmed != "**" && trimmed != "**/*" {
                for (graph_file, &h) in hop_map {
                    if matches_path_glob(trimmed, graph_file) {
                        update_min(h);
                    }
                }
            }
        }

        min_hops
    }

    /// Determines the minimum import hop distance between a `FailurePattern` and the active file hop map.
    pub fn min_hop_distance_for_failure(
        &self,
        failure: &FailurePattern,
        hop_map: &HashMap<String, usize>,
    ) -> Option<usize> {
        if hop_map.is_empty() {
            return None;
        }

        let mut min_hops: Option<usize> = None;
        let mut update_min = |h: usize| {
            min_hops = Some(match min_hops {
                Some(curr) => curr.min(h),
                None => h,
            });
        };

        for file in Self::extract_failure_files(failure) {
            if let Some(&h) = hop_map.get(&file) {
                update_min(h);
            } else {
                for (graph_file, &h) in hop_map {
                    if paths_match_suffix(graph_file, &file) {
                        update_min(h);
                    }
                }
            }
        }

        if let Some(glob) = failure.path_glob() {
            let trimmed = glob.trim();
            if !trimmed.is_empty() && trimmed != "*" && trimmed != "**" && trimmed != "**/*" {
                for (graph_file, &h) in hop_map {
                    if matches_path_glob(trimmed, graph_file) {
                        update_min(h);
                    }
                }
            }
        }

        min_hops
    }

    /// Computes the decay factor (`1.5` for hop 0, `1.0` for hop 1, `0.5` for hop 2, `0.0` otherwise)
    /// for a `MemoryRecord` given `active_files`.
    pub fn memory_decay_factor(&self, record: &MemoryRecord, active_files: &[String]) -> f32 {
        let hop_map = self.compute_hop_distances(active_files);
        self.memory_decay_factor_with_map(record, &hop_map)
    }

    pub fn memory_decay_factor_with_map(
        &self,
        record: &MemoryRecord,
        hop_map: &HashMap<String, usize>,
    ) -> f32 {
        match self.min_hop_distance_for_memory(record, hop_map) {
            Some(hops) => self.decay_factor_for_hops(hops),
            None => 0.0,
        }
    }

    /// Computes the weighted proximity score boost for a `MemoryRecord`.
    pub fn memory_proximity_boost(&self, record: &MemoryRecord, active_files: &[String]) -> f32 {
        self.config.proximity_weight * self.memory_decay_factor(record, active_files)
    }

    pub fn memory_proximity_boost_with_map(
        &self,
        record: &MemoryRecord,
        hop_map: &HashMap<String, usize>,
    ) -> f32 {
        self.config.proximity_weight * self.memory_decay_factor_with_map(record, hop_map)
    }

    /// Computes the weighted proximity score boost for a `FailurePattern`.
    pub fn failure_proximity_boost(
        &self,
        failure: &FailurePattern,
        active_files: &[String],
    ) -> f32 {
        let hop_map = self.compute_hop_distances(active_files);
        self.failure_proximity_boost_with_map(failure, &hop_map)
    }

    pub fn failure_proximity_boost_with_map(
        &self,
        failure: &FailurePattern,
        hop_map: &HashMap<String, usize>,
    ) -> f32 {
        match self.min_hop_distance_for_failure(failure, hop_map) {
            Some(hops) => self.config.proximity_weight * self.decay_factor_for_hops(hops),
            None => 0.0,
        }
    }
}

fn paths_match_suffix(a: &str, b: &str) -> bool {
    if a == b {
        return true;
    }
    if a.len() > b.len() && a.ends_with(b) && a.as_bytes()[a.len() - b.len() - 1] == b'/' {
        return true;
    }
    if b.len() > a.len() && b.ends_with(a) && b.as_bytes()[b.len() - a.len() - 1] == b'/' {
        return true;
    }
    false
}

fn looks_like_file_path(s: &str) -> bool {
    let cleaned = s.trim_matches(|c: char| {
        !c.is_alphanumeric() && c != '_' && c != '-' && c != '/' && c != '\\' && c != '.'
    });
    if cleaned.is_empty() || cleaned.contains("::") || cleaned.contains("..") {
        return false;
    }
    const EXTS: &[&str] = &[
        ".rs", ".ts", ".tsx", ".js", ".jsx", ".py", ".go", ".java", ".c", ".cpp", ".h", ".hpp",
        ".toml", ".json", ".yaml", ".yml", ".md", ".sql",
    ];
    EXTS.iter().any(|ext| cleaned.ends_with(ext))
}

fn extract_inline_file_paths(text: &str) -> Vec<String> {
    let mut results = Vec::new();
    for word in text.split_whitespace() {
        let cleaned = word.trim_matches(|c: char| {
            !c.is_alphanumeric() && c != '_' && c != '-' && c != '/' && c != '\\' && c != '.'
        });
        if looks_like_file_path(cleaned) {
            results.push(ProximityBiasEngine::normalize_file_path(cleaned));
        }
    }
    results
}

#[derive(Debug, Clone)]
pub struct HybridRankerConfig {
    /// RRF smoothing constant (default: 60.0)
    pub rrf_k: f32,
    /// Lexical BM25 ranker weight (default: 0.4)
    pub bm25_weight: f32,
    /// Vector cosine similarity ranker weight (default: 0.4)
    pub vector_weight: f32,
    /// Graph spreading activation ranker weight (default: 0.35)
    pub graph_weight: f32,
    /// Minimum cosine similarity threshold for vector candidates (default: 0.0)
    pub min_similarity: f32,
    /// Enable associative Knowledge Graph spreading activation retrieval
    pub enable_spreading_activation: bool,
    /// Spreading activation engine configuration
    pub activation_config: SpreadingActivationConfig,
    /// Priority weighting for federated global store matches (default: 0.8)
    pub global_federation_weight: f32,
    /// Minimum memories required in a community before strictly isolating recall (default: 5).
    /// If fewer than this exist, falls back to global recall to avoid cold-start starvation.
    pub min_community_memories_for_isolation: usize,
    /// Whether explicit bridge dependencies are included when scoped to a community (default: true)
    pub include_cross_cluster_bridges: bool,
    /// Active file & cursor proximity biasing configuration
    pub proximity_config: ProximityBiasConfig,
}

impl Default for HybridRankerConfig {
    fn default() -> Self {
        Self {
            rrf_k: 60.0,
            bm25_weight: 0.4,
            vector_weight: 0.4,
            graph_weight: 0.35,
            min_similarity: 0.0,
            enable_spreading_activation: true,
            activation_config: SpreadingActivationConfig::default(),
            global_federation_weight: 0.8,
            min_community_memories_for_isolation: 5,
            include_cross_cluster_bridges: true,
            proximity_config: ProximityBiasConfig::default(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct HybridRanker {
    config: HybridRankerConfig,
    spreading_engine: SpreadingActivationEngine,
}

impl HybridRanker {
    pub fn new(config: HybridRankerConfig) -> Self {
        let spreading_engine = SpreadingActivationEngine::new(config.activation_config.clone());
        Self {
            config,
            spreading_engine,
        }
    }

    pub fn with_default_config() -> Self {
        Self::new(HybridRankerConfig::default())
    }

    pub fn config(&self) -> &HybridRankerConfig {
        &self.config
    }

    pub fn spreading_engine(&self) -> &SpreadingActivationEngine {
        &self.spreading_engine
    }

    /// Perform hybrid retrieval combining FTS5 BM25, Vector Cosine Similarity,
    /// and Knowledge Graph Spreading Activation via Tri-Modal RRF.
    /// Transparently falls back to global recall if community scope has < k memories.
    pub async fn retrieve(
        &self,
        store: &SqliteStore,
        embedding_provider: Option<&dyn EmbeddingProvider>,
        query: &str,
        scope: Option<&Scope>,
        memory_types: Option<&[MemoryType]>,
        limit: usize,
    ) -> Result<Vec<MemoryRecord>, StrataError> {
        self.retrieve_as_of(
            store,
            embedding_provider,
            query,
            scope,
            memory_types,
            None,
            None,
            None,
            limit,
        )
        .await
    }

    /// Perform hybrid retrieval biased by active files and their 1-hop / 2-hop import graph neighbors.
    pub async fn retrieve_with_active_files(
        &self,
        store: &SqliteStore,
        embedding_provider: Option<&dyn EmbeddingProvider>,
        query: &str,
        scope: Option<&Scope>,
        memory_types: Option<&[MemoryType]>,
        active_files: &[String],
        limit: usize,
    ) -> Result<Vec<MemoryRecord>, StrataError> {
        self.retrieve_as_of_with_active_files(
            store,
            embedding_provider,
            query,
            scope,
            memory_types,
            None,
            None,
            None,
            active_files,
            limit,
        )
        .await
    }

    /// Perform hybrid retrieval combining FTS5 BM25, Vector Cosine Similarity,
    /// and Knowledge Graph Spreading Activation via Tri-Modal RRF with bi-temporal and branch filters.
    pub async fn retrieve_as_of(
        &self,
        store: &SqliteStore,
        embedding_provider: Option<&dyn EmbeddingProvider>,
        query: &str,
        scope: Option<&Scope>,
        memory_types: Option<&[MemoryType]>,
        as_of_time: Option<DateTime<Utc>>,
        as_of_commit: Option<&str>,
        as_of_branch: Option<&str>,
        limit: usize,
    ) -> Result<Vec<MemoryRecord>, StrataError> {
        self.retrieve_as_of_with_active_files(
            store,
            embedding_provider,
            query,
            scope,
            memory_types,
            as_of_time,
            as_of_commit,
            as_of_branch,
            &[],
            limit,
        )
        .await
    }

    /// Perform hybrid retrieval with bi-temporal, branch, and active file proximity biasing.
    pub async fn retrieve_as_of_with_active_files(
        &self,
        store: &SqliteStore,
        embedding_provider: Option<&dyn EmbeddingProvider>,
        query: &str,
        scope: Option<&Scope>,
        memory_types: Option<&[MemoryType]>,
        as_of_time: Option<DateTime<Utc>>,
        as_of_commit: Option<&str>,
        as_of_branch: Option<&str>,
        active_files: &[String],
        limit: usize,
    ) -> Result<Vec<MemoryRecord>, StrataError> {
        let candidate_limit = (limit * 3).max(20);

        // Transparent cold-start fallback if community has < k memories
        let effective_scope = if let Some(Scope::Community(ref c_id)) = scope {
            let count = store.count_memories_by_scope(&Scope::Community(c_id.clone()))?;
            if count < self.config.min_community_memories_for_isolation {
                None
            } else {
                scope
            }
        } else {
            scope
        };

        // 1. Lexical retrieval via SQLite FTS5 with bi-temporal bounds
        let fts_results = store.search_fts_as_of(
            query,
            effective_scope,
            as_of_time,
            as_of_commit,
            as_of_branch,
            candidate_limit,
        )?;

        // 2. Vector retrieval filtered by bi-temporal bounds
        let mut vector_results: Vec<(MemoryRecord, f32)> = Vec::new();
        if let Some(embedder) = embedding_provider {
            if let Ok(query_embedding) = embedder.embed_text(query).await {
                // Fetch candidates in scope to score against
                let all_candidates = store.get_all_memories_as_of(
                    effective_scope,
                    memory_types,
                    as_of_time,
                    as_of_commit,
                    as_of_branch,
                    candidate_limit * 2,
                )?;
                for memory in all_candidates {
                    if !is_embedding_compatible_with_query(
                        embedder.model_name(),
                        memory.embedding_model.as_deref(),
                    ) {
                        continue;
                    }
                    if let Some(ref mem_emb) = memory.embedding {
                        let sim = cosine_similarity(&query_embedding, mem_emb);
                        if sim >= self.config.min_similarity {
                            vector_results.push((memory, sim));
                        }
                    }
                }
                // Sort vector results by similarity descending
                vector_results
                    .sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
            }
        }

        // 3. Associative Graph Spreading Activation retrieval (HippoRAG style)
        let mut graph_results: Vec<(MemoryRecord, f32)> = Vec::new();
        if self.config.enable_spreading_activation {
            if let Ok(graph) = self
                .spreading_engine
                .build_graph_from_store(store, effective_scope)
            {
                // Build initial seed activations
                let mut seeds = self.spreading_engine.seed_from_query(&graph, query);

                // Seed active files in the associative graph as well
                for af in active_files {
                    let norm_af = ProximityBiasEngine::normalize_file_path(af);
                    if !norm_af.is_empty() {
                        for matched in graph.find_matching_nodes(&norm_af) {
                            seeds.push((matched, 1.0));
                        }
                    }
                }

                // Add top FTS candidates as memory seeds
                for (rec, fts_score) in fts_results.iter().take(5) {
                    let norm_score = (1.0 / (1.0 + fts_score.abs())).clamp(0.2, 1.0);
                    seeds.push((GraphNode::Memory(rec.id), norm_score));
                }

                // Add top vector candidates as memory seeds
                for (rec, sim) in vector_results.iter().take(5) {
                    seeds.push((GraphNode::Memory(rec.id), *sim));
                }

                if !seeds.is_empty() {
                    let activations = self.spreading_engine.propagate(&graph, &seeds);
                    let top_memories = self
                        .spreading_engine
                        .extract_top_memories(&activations, candidate_limit);

                    // Collect records for activated memory IDs
                    for (mem_id, act_score) in top_memories {
                        // Check if already fetched in FTS or vector results
                        if let Some((rec, _)) = fts_results.iter().find(|(r, _)| r.id == mem_id) {
                            graph_results.push((rec.clone(), act_score));
                        } else if let Some((rec, _)) =
                            vector_results.iter().find(|(r, _)| r.id == mem_id)
                        {
                            graph_results.push((rec.clone(), act_score));
                        } else if let Ok(Some(rec)) =
                            store.get_memory_as_of(&mem_id, as_of_time, as_of_commit, as_of_branch)
                        {
                            // Discovered via associative multi-hop jump!
                            if effective_scope.map_or(true, |s| s.is_compatible(&rec.scope))
                                && memory_types
                                    .map_or(true, |types| types.contains(&rec.memory_type))
                            {
                                graph_results.push((rec, act_score));
                            }
                        }
                    }
                }
            }
        }

        let proximity_engine = if !active_files.is_empty() {
            ProximityBiasEngine::from_store(store, self.config.proximity_config.clone()).ok()
        } else {
            None
        };

        // If all candidate streams are empty, fall back to recent memories only if query is blank
        if fts_results.is_empty() && vector_results.is_empty() && graph_results.is_empty() {
            if query.trim().is_empty() {
                let mut candidates = store.get_all_memories_as_of(
                    effective_scope,
                    memory_types,
                    as_of_time,
                    as_of_commit,
                    as_of_branch,
                    candidate_limit,
                )?;
                candidates.retain(|rec| {
                    let is_suppressed = rec
                        .metadata
                        .get("quarantined")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false)
                        || rec
                            .metadata
                            .get("suppressed")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(false)
                        || rec
                            .metadata
                            .get("status")
                            .and_then(|v| v.as_str())
                            .map_or(false, |s| {
                                s.eq_ignore_ascii_case("quarantined")
                                    || s.eq_ignore_ascii_case("suppressed")
                            });
                    !is_suppressed
                });
                if let Some(ref prox) = proximity_engine {
                    let hop_map = prox.compute_hop_distances(active_files);
                    candidates.sort_by(|a, b| {
                        let boost_a = prox.memory_proximity_boost_with_map(a, &hop_map);
                        let boost_b = prox.memory_proximity_boost_with_map(b, &hop_map);
                        let score_a = a.importance * a.confidence + boost_a;
                        let score_b = b.importance * b.confidence + boost_b;
                        score_b
                            .partial_cmp(&score_a)
                            .unwrap_or(std::cmp::Ordering::Equal)
                    });
                }
                candidates.truncate(limit);
                return Ok(candidates);
            } else {
                return Ok(Vec::new());
            }
        }

        // 4. Compute Tri-Modal Reciprocal Rank Fusion (RRF) with optional active file proximity bias
        let fused = self.fuse_tri_ranks_with_proximity(
            &fts_results,
            &vector_results,
            &graph_results,
            proximity_engine.as_ref(),
            active_files,
            limit,
        );
        Ok(fused)
    }

    /// Backward-compatible dual-modal fusion (BM25 + Vector).
    pub fn fuse_ranks(
        &self,
        fts_ranked: &[(MemoryRecord, f32)],
        vector_ranked: &[(MemoryRecord, f32)],
        limit: usize,
    ) -> Vec<MemoryRecord> {
        self.fuse_tri_ranks(fts_ranked, vector_ranked, &[], limit)
    }

    /// Fuse BM25, Vector, and Spreading Activation rankings using Tri-Modal Reciprocal Rank Fusion (RRF).
    pub fn fuse_tri_ranks(
        &self,
        fts_ranked: &[(MemoryRecord, f32)],
        vector_ranked: &[(MemoryRecord, f32)],
        graph_ranked: &[(MemoryRecord, f32)],
        limit: usize,
    ) -> Vec<MemoryRecord> {
        self.fuse_tri_ranks_with_proximity(fts_ranked, vector_ranked, graph_ranked, None, &[], limit)
    }

    /// Fuse BM25, Vector, and Spreading Activation rankings using Tri-Modal RRF,
    /// augmented by Active File & Cursor Proximity Biasing.
    pub fn fuse_tri_ranks_with_proximity(
        &self,
        fts_ranked: &[(MemoryRecord, f32)],
        vector_ranked: &[(MemoryRecord, f32)],
        graph_ranked: &[(MemoryRecord, f32)],
        proximity_engine: Option<&ProximityBiasEngine>,
        active_files: &[String],
        limit: usize,
    ) -> Vec<MemoryRecord> {
        let mut score_map: HashMap<Uuid, f32> = HashMap::new();
        let mut record_map: HashMap<Uuid, MemoryRecord> = HashMap::new();

        let k = self.config.rrf_k;
        let w_bm25 = self.config.bm25_weight;
        let w_vec = self.config.vector_weight;
        let w_graph = self.config.graph_weight;

        // Score FTS results
        for (rank_idx, (record, _bm25_score)) in fts_ranked.iter().enumerate() {
            let rrf_score = w_bm25 * (1.0 / (k + (rank_idx as f32) + 1.0));
            *score_map.entry(record.id).or_insert(0.0) += rrf_score;
            record_map
                .entry(record.id)
                .or_insert_with(|| record.clone());
        }

        // Score Vector results
        for (rank_idx, (record, _sim)) in vector_ranked.iter().enumerate() {
            let rrf_score = w_vec * (1.0 / (k + (rank_idx as f32) + 1.0));
            *score_map.entry(record.id).or_insert(0.0) += rrf_score;
            record_map
                .entry(record.id)
                .or_insert_with(|| record.clone());
        }

        // Score Knowledge Graph Spreading Activation results
        for (rank_idx, (record, _act_score)) in graph_ranked.iter().enumerate() {
            let rrf_score = w_graph * (1.0 / (k + (rank_idx as f32) + 1.0));
            *score_map.entry(record.id).or_insert(0.0) += rrf_score;
            record_map
                .entry(record.id)
                .or_insert_with(|| record.clone());
        }

        let hop_map = proximity_engine
            .filter(|_| !active_files.is_empty())
            .map(|prox| prox.compute_hop_distances(active_files));

        // Apply quality weighting (importance & confidence) and active file proximity boost
        for (id, score) in score_map.iter_mut() {
            if let Some(rec) = record_map.get(id) {
                let is_suppressed = rec
                    .metadata
                    .get("quarantined")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false)
                    || rec
                        .metadata
                        .get("suppressed")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false)
                    || rec
                        .metadata
                        .get("status")
                        .and_then(|v| v.as_str())
                        .map_or(false, |s| {
                            s.eq_ignore_ascii_case("quarantined")
                                || s.eq_ignore_ascii_case("suppressed")
                        });

                if is_suppressed {
                    *score = 0.0;
                    continue;
                }

                let importance_factor = 0.8 + 0.4 * rec.importance;
                let confidence_factor = 0.5 + 0.5 * rec.confidence;
                *score *= importance_factor * confidence_factor;

                if let (Some(prox), Some(ref hmap)) = (proximity_engine, &hop_map) {
                    let proximity_boost = prox.memory_proximity_boost_with_map(rec, hmap);
                    *score += proximity_boost;
                }
            }
        }

        let mut ranked_ids: Vec<(Uuid, f32)> = score_map.into_iter().collect();
        ranked_ids.retain(|(_, score)| *score > 0.0);
        ranked_ids.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        ranked_ids
            .into_iter()
            .take(limit)
            .filter_map(|(id, _)| record_map.remove(&id))
            .collect()
    }

    /// Perform federated hybrid retrieval across a local workspace store and an optional global store.
    /// Results are ranked using Tri-Modal RRF with priority weighting (local 1.0 vs global_federation_weight).
    pub async fn retrieve_federated(
        &self,
        local_store: &SqliteStore,
        global_store: Option<&SqliteStore>,
        embedding_provider: Option<&dyn EmbeddingProvider>,
        query: &str,
        scope: Option<&Scope>,
        memory_types: Option<&[MemoryType]>,
        limit: usize,
    ) -> Result<Vec<MemoryRecord>, StrataError> {
        self.retrieve_federated_as_of(
            local_store,
            global_store,
            embedding_provider,
            query,
            scope,
            memory_types,
            None,
            None,
            None,
            limit,
        )
        .await
    }

    /// Perform federated hybrid retrieval across a local workspace store and an optional global store with bi-temporal and branch filters.
    pub async fn retrieve_federated_as_of(
        &self,
        local_store: &SqliteStore,
        global_store: Option<&SqliteStore>,
        embedding_provider: Option<&dyn EmbeddingProvider>,
        query: &str,
        scope: Option<&Scope>,
        memory_types: Option<&[MemoryType]>,
        as_of_time: Option<DateTime<Utc>>,
        as_of_commit: Option<&str>,
        as_of_branch: Option<&str>,
        limit: usize,
    ) -> Result<Vec<MemoryRecord>, StrataError> {
        let global = match global_store {
            Some(g) => g,
            None => {
                return self
                    .retrieve_as_of(
                        local_store,
                        embedding_provider,
                        query,
                        scope,
                        memory_types,
                        as_of_time,
                        as_of_commit,
                        as_of_branch,
                        limit,
                    )
                    .await;
            }
        };

        // Fetch local candidates
        let candidate_limit = (limit * 2).max(10);
        let local_results = self
            .retrieve_as_of(
                local_store,
                embedding_provider,
                query,
                scope,
                memory_types,
                as_of_time,
                as_of_commit,
                as_of_branch,
                candidate_limit,
            )
            .await?;

        // Fetch global candidates (scoped to Global, cross-branch canonical)
        let global_results = self
            .retrieve_as_of(
                global,
                embedding_provider,
                query,
                Some(&Scope::Global),
                memory_types,
                as_of_time,
                as_of_commit,
                None,
                candidate_limit,
            )
            .await?;

        if global_results.is_empty() {
            let mut res = local_results;
            res.truncate(limit);
            return Ok(res);
        }

        if local_results.is_empty() {
            let mut res = global_results;
            res.truncate(limit);
            return Ok(res);
        }

        // Blend local and global candidate ranks via weighted RRF
        let fused = self.fuse_federated_candidates(&local_results, &global_results, limit);
        Ok(fused)
    }

    /// Blend local and global candidate lists using RRF with scope-based weighting.
    pub fn fuse_federated_candidates(
        &self,
        local_ranked: &[MemoryRecord],
        global_ranked: &[MemoryRecord],
        limit: usize,
    ) -> Vec<MemoryRecord> {
        let mut score_map: HashMap<Uuid, f32> = HashMap::new();
        let mut record_map: HashMap<Uuid, MemoryRecord> = HashMap::new();

        let k = self.config.rrf_k;
        let w_local = 1.0;
        let w_global = self.config.global_federation_weight;

        // Local candidates
        for (rank_idx, record) in local_ranked.iter().enumerate() {
            let rrf_score = w_local * (1.0 / (k + (rank_idx as f32) + 1.0));
            *score_map.entry(record.id).or_insert(0.0) += rrf_score;
            record_map
                .entry(record.id)
                .or_insert_with(|| record.clone());
        }

        // Global candidates
        for (rank_idx, record) in global_ranked.iter().enumerate() {
            let rrf_score = w_global * (1.0 / (k + (rank_idx as f32) + 1.0));
            *score_map.entry(record.id).or_insert(0.0) += rrf_score;
            record_map
                .entry(record.id)
                .or_insert_with(|| record.clone());
        }

        // Apply quality weighting: importance & confidence
        for (id, score) in score_map.iter_mut() {
            if let Some(rec) = record_map.get(id) {
                let is_suppressed = rec
                    .metadata
                    .get("quarantined")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false)
                    || rec
                        .metadata
                        .get("suppressed")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false)
                    || rec
                        .metadata
                        .get("status")
                        .and_then(|v| v.as_str())
                        .map_or(false, |s| {
                            s.eq_ignore_ascii_case("quarantined")
                                || s.eq_ignore_ascii_case("suppressed")
                        });

                if is_suppressed {
                    *score = 0.0;
                    continue;
                }

                let importance_factor = 0.8 + 0.4 * rec.importance;
                let confidence_factor = 0.5 + 0.5 * rec.confidence;
                *score *= importance_factor * confidence_factor;
            }
        }

        let mut ranked_ids: Vec<(Uuid, f32)> = score_map.into_iter().collect();
        ranked_ids.retain(|(_, score)| *score > 0.0);
        ranked_ids.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        ranked_ids
            .into_iter()
            .take(limit)
            .filter_map(|(id, _)| record_map.remove(&id))
            .collect()
    }

    /// Performs community-scoped hybrid retrieval using the CommunityMemoryRouter.
    /// Provides candidate reduction statistics, bridge expansion, and transparent fallback.
    pub async fn retrieve_community_scoped(
        &self,
        store: &SqliteStore,
        embedding_provider: Option<&dyn EmbeddingProvider>,
        query: &str,
        community_id: &str,
        summary: Option<&crate::community::ArchitectureGraphSummary>,
        memory_types: Option<&[MemoryType]>,
        limit: usize,
    ) -> Result<crate::community::CommunityScopedRecallResult, StrataError> {
        let router = crate::community::CommunityMemoryRouter::new(
            self.config.min_community_memories_for_isolation,
            self.config.include_cross_cluster_bridges,
        );
        router
            .retrieve_scoped(
                self,
                store,
                embedding_provider,
                query,
                community_id,
                summary,
                memory_types,
                limit,
            )
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::call_graph::{CallEdge, CallType};
    use crate::embedding::MockEmbeddingProvider;
    use crate::SqliteMemoryEngine;
    use strata_core::traits::MemoryEngine;

    #[test]
    fn test_import_distance_decay_factor() {
        // Hop 0 (active file itself) >= 1.0
        assert!(import_distance_decay_factor(0) > 1.0);
        // Hop 1 (direct import) = 1.0
        assert!((import_distance_decay_factor(1) - 1.0).abs() < f32::EPSILON);
        // Hop 2 (2-hop import) = 0.5
        assert!((import_distance_decay_factor(2) - 0.5).abs() < f32::EPSILON);
        // Hop >= 3 = 0.0
        assert!((import_distance_decay_factor(3) - 0.0).abs() < f32::EPSILON);
        assert!((import_distance_decay_factor(10) - 0.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_proximity_bias_engine_hop_distances_and_decay() {
        // Build import chain: src/app.rs -> src/auth.rs (hop 1) -> src/crypto.rs (hop 2) -> src/primitives.rs (hop 3)
        // And unconnected: src/billing.rs
        let edges = vec![
            CallEdge::new("src/app.rs", "<top-level>", "crate::auth", 1, CallType::Import)
                .with_callee_file("src/auth.rs"),
            CallEdge::new("src/auth.rs", "<top-level>", "crate::crypto", 2, CallType::Import)
                .with_callee_file("src/crypto.rs"),
            CallEdge::new(
                "src/crypto.rs",
                "<top-level>",
                "crate::primitives",
                3,
                CallType::Import,
            )
            .with_callee_file("src/primitives.rs"),
        ];

        let engine = ProximityBiasEngine::from_call_edges(&edges, ProximityBiasConfig::default());
        let active_files = vec!["./src\\app.rs".to_string()];
        let hop_map = engine.compute_hop_distances(&active_files);

        assert_eq!(hop_map.get("src/app.rs"), Some(&0));
        assert_eq!(hop_map.get("src/auth.rs"), Some(&1));
        assert_eq!(hop_map.get("src/crypto.rs"), Some(&2));
        // Hop 3 exceeds max_import_hops (2)
        assert_eq!(hop_map.get("src/primitives.rs"), None);
        assert_eq!(hop_map.get("src/billing.rs"), None);

        let make_mem = |file: &str| {
            let mut rec = MemoryRecord::new(
                MemoryType::Semantic,
                format!("Security invariant for {file}"),
                Scope::Global,
            );
            rec.metadata = serde_json::json!({ "file_path": file });
            rec
        };

        let mem_app = make_mem("src/app.rs");
        let mem_auth = make_mem("src/auth.rs");
        let mem_crypto = make_mem("src/crypto.rs");
        let mem_billing = make_mem("src/billing.rs");

        assert!((engine.memory_decay_factor(&mem_app, &active_files) - 1.5).abs() < f32::EPSILON);
        assert!((engine.memory_decay_factor(&mem_auth, &active_files) - 1.0).abs() < f32::EPSILON);
        assert!(
            (engine.memory_decay_factor(&mem_crypto, &active_files) - 0.5).abs() < f32::EPSILON
        );
        assert!(
            (engine.memory_decay_factor(&mem_billing, &active_files) - 0.0).abs() < f32::EPSILON
        );
    }

    #[tokio::test]
    async fn test_retrieve_with_active_files_orders_by_import_proximity() {
        let store = SqliteStore::open_in_memory().expect("open store");
        let embedder = MockEmbeddingProvider::default();

        // Persist import graph: src/app.rs -> src/auth.rs (hop 1) -> src/crypto.rs (hop 2)
        let edges = vec![
            CallEdge::new("src/app.rs", "handle_req", "verify_jwt", 10, CallType::FunctionCall)
                .with_callee_file("src/auth.rs"),
            CallEdge::new("src/auth.rs", "verify_jwt", "hmac_sha256", 25, CallType::FunctionCall)
                .with_callee_file("src/crypto.rs"),
        ];
        store.save_call_edges(&edges).expect("save call edges");

        // Create 4 memories with identical base importance, confidence, and lexical match on "validation"
        let make_record = |label: &str, file: &str| {
            let mut rec = MemoryRecord::new(
                MemoryType::Semantic,
                format!("Token validation invariant rule for module {label}"),
                Scope::Global,
            )
            .with_importance(0.7)
            .with_confidence(0.9);
            rec.metadata = serde_json::json!({ "file_path": file });
            rec
        };

        // Insert in reverse order (unconnected first, then hop 2, hop 1, hop 0)
        let mem_unconnected = make_record("billing", "src/billing.rs");
        let mem_hop2 = make_record("crypto", "src/crypto.rs");
        let mem_hop1 = make_record("auth", "src/auth.rs");
        let mem_hop0 = make_record("app", "src/app.rs");

        for m in [&mem_unconnected, &mem_hop2, &mem_hop1, &mem_hop0] {
            let mut with_emb = m.clone();
            with_emb.embedding = Some(embedder.embed_text(&with_emb.content).await.unwrap());
            store.insert_or_update_memory(&with_emb).unwrap();
        }

        let ranker = HybridRanker::with_default_config();
        let active_files = vec!["src/app.rs".to_string()];

        let results = ranker
            .retrieve_with_active_files(
                &store,
                Some(&embedder),
                "Token validation invariant",
                Some(&Scope::Global),
                None,
                &active_files,
                4,
            )
            .await
            .expect("retrieve_with_active_files");

        assert_eq!(results.len(), 4);
        assert_eq!(
            results[0].id, mem_hop0.id,
            "Active file (hop 0) must rank first"
        );
        assert_eq!(
            results[1].id, mem_hop1.id,
            "1-hop import (factor 1.0) must rank second"
        );
        assert_eq!(
            results[2].id, mem_hop2.id,
            "2-hop import (factor 0.5) must rank third"
        );
        assert_eq!(
            results[3].id, mem_unconnected.id,
            "Unconnected file (factor 0.0) must rank fourth"
        );
    }

    #[tokio::test]
    async fn test_engine_recall_and_digest_with_active_files() {
        let engine = SqliteMemoryEngine::open_in_memory(None).expect("open engine");
        let session_id = "sess-prox-1";
        let scope = Scope::Session(session_id.to_string());

        let edges = vec![
            CallEdge::new("src/main.rs", "main", "init_db", 5, CallType::FunctionCall)
                .with_callee_file("src/db.rs"),
            CallEdge::new("src/db.rs", "init_db", "open_pool", 12, CallType::FunctionCall)
                .with_callee_file("src/pool.rs"),
        ];
        engine.store().save_call_edges(&edges).unwrap();

        let make_mem = |title: &str, file: &str| {
            let mut rec = MemoryRecord::new(
                MemoryType::Semantic,
                format!("Database connection guideline for {title}"),
                scope.clone(),
            )
            .with_summary(title.to_string())
            .with_importance(0.6)
            .with_confidence(0.9);
            rec.metadata = serde_json::json!({ "file_path": file });
            rec
        };

        let m_unrelated = make_mem("Unrelated UI guideline", "src/ui.rs");
        let m_hop2 = make_mem("Pool guideline (hop 2)", "src/pool.rs");
        let m_hop1 = make_mem("DB guideline (hop 1)", "src/db.rs");
        let m_hop0 = make_mem("Main entrypoint guideline (hop 0)", "src/main.rs");

        for m in [&m_unrelated, &m_hop2, &m_hop1, &m_hop0] {
            engine.write(m).await.unwrap();
        }

        let active = vec!["src/main.rs".to_string()];

        // 1. Verify recall_with_active_files
        let recalled = engine
            .recall_with_active_files("Database connection guideline", Some(&scope), &active, 4)
            .await
            .expect("recall_with_active_files");
        assert_eq!(recalled.len(), 4);
        assert_eq!(recalled[0].id, m_hop0.id);
        assert_eq!(recalled[1].id, m_hop1.id);
        assert_eq!(recalled[2].id, m_hop2.id);
        assert_eq!(recalled[3].id, m_unrelated.id);

        // 2. Verify memory_digest_with_active_files orders key_pointers by proximity
        let digest = engine
            .memory_digest_with_active_files(session_id, Some(500), &active)
            .await
            .expect("memory_digest_with_active_files");
        assert_eq!(digest.key_pointers.len(), 4);
        assert_eq!(digest.key_pointers[0].id, m_hop0.id);
        assert_eq!(digest.key_pointers[1].id, m_hop1.id);
        assert_eq!(digest.key_pointers[2].id, m_hop2.id);
        assert_eq!(digest.key_pointers[3].id, m_unrelated.id);
    }
}

