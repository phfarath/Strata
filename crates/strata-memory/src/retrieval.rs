use std::collections::HashMap;
use strata_core::errors::StrataError;
use strata_core::state::{MemoryRecord, MemoryType, Scope};
use uuid::Uuid;

use crate::embedding::{cosine_similarity, EmbeddingProvider};
use crate::spreading_activation::{
    GraphNode, SpreadingActivationConfig, SpreadingActivationEngine,
};
use crate::store::SqliteStore;

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
    pub async fn retrieve(
        &self,
        store: &SqliteStore,
        embedding_provider: Option<&dyn EmbeddingProvider>,
        query: &str,
        scope: Option<&Scope>,
        memory_types: Option<&[MemoryType]>,
        limit: usize,
    ) -> Result<Vec<MemoryRecord>, StrataError> {
        let candidate_limit = (limit * 3).max(20);

        // 1. Lexical retrieval via SQLite FTS5
        let fts_results = store.search_fts(query, scope, candidate_limit)?;

        // 2. Vector retrieval
        let mut vector_results: Vec<(MemoryRecord, f32)> = Vec::new();
        if let Some(embedder) = embedding_provider {
            if let Ok(query_embedding) = embedder.embed_text(query).await {
                // Fetch candidates in scope to score against
                let all_candidates =
                    store.get_all_memories(scope, memory_types, candidate_limit * 2)?;
                for memory in all_candidates {
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
            if let Ok(graph) = self.spreading_engine.build_graph_from_store(store, scope) {
                // Build initial seed activations
                let mut seeds = self.spreading_engine.seed_from_query(&graph, query);

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
                        } else if let Ok(Some(rec)) = store.get_memory(&mem_id) {
                            // Discovered via associative multi-hop jump!
                            if scope.map_or(true, |s| s.is_compatible(&rec.scope))
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

        // If all candidate streams are empty, fall back to recent memories only if query is blank
        if fts_results.is_empty() && vector_results.is_empty() && graph_results.is_empty() {
            if query.trim().is_empty() {
                return store.get_all_memories(scope, memory_types, limit);
            } else {
                return Ok(Vec::new());
            }
        }

        // 4. Compute Tri-Modal Reciprocal Rank Fusion (RRF)
        let fused = self.fuse_tri_ranks(&fts_results, &vector_results, &graph_results, limit);
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

        // Apply quality weighting: importance & confidence
        for (id, score) in score_map.iter_mut() {
            if let Some(rec) = record_map.get(id) {
                let importance_factor = 0.8 + 0.4 * rec.importance;
                let confidence_factor = 0.5 + 0.5 * rec.confidence;
                *score *= importance_factor * confidence_factor;
            }
        }

        let mut ranked_ids: Vec<(Uuid, f32)> = score_map.into_iter().collect();
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
        let global = match global_store {
            Some(g) => g,
            None => {
                return self
                    .retrieve(
                        local_store,
                        embedding_provider,
                        query,
                        scope,
                        memory_types,
                        limit,
                    )
                    .await;
            }
        };

        // Fetch local candidates
        let candidate_limit = (limit * 2).max(10);
        let local_results = self
            .retrieve(
                local_store,
                embedding_provider,
                query,
                scope,
                memory_types,
                candidate_limit,
            )
            .await?;

        // Fetch global candidates (scoped to Global)
        let global_results = self
            .retrieve(
                global,
                embedding_provider,
                query,
                Some(&Scope::Global),
                memory_types,
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
                let importance_factor = 0.8 + 0.4 * rec.importance;
                let confidence_factor = 0.5 + 0.5 * rec.confidence;
                *score *= importance_factor * confidence_factor;
            }
        }

        let mut ranked_ids: Vec<(Uuid, f32)> = score_map.into_iter().collect();
        ranked_ids.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        ranked_ids
            .into_iter()
            .take(limit)
            .filter_map(|(id, _)| record_map.remove(&id))
            .collect()
    }
}
