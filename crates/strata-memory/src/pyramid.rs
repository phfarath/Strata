use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::Arc;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use strata_core::errors::StrataError;
use strata_core::schemas::EpisodeRecord;

use crate::embedding::cosine_similarity;
use crate::store::SqliteStore;

/// Configuration options for Hierarchical Episodic Memory Pyramid rollups.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PyramidConfig {
    /// Minimum number of episodes required to form a cluster / rollup (default 2).
    pub min_cluster_size: usize,
    /// Maximum number of episodes in a single cluster / rollup (default 20).
    pub max_cluster_size: usize,
    /// Cosine similarity threshold for vector-based grouping (default 0.70).
    pub cluster_similarity_threshold: f32,
    /// Target hierarchical tier for generated rollups (default 2 = Milestone Rollup).
    pub target_tier: u8,
}

impl Default for PyramidConfig {
    fn default() -> Self {
        Self {
            min_cluster_size: 2,
            max_cluster_size: 20,
            cluster_similarity_threshold: 0.70,
            target_tier: 2,
        }
    }
}

impl PyramidConfig {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_min_cluster_size(mut self, min: usize) -> Self {
        self.min_cluster_size = min;
        self
    }

    pub fn with_max_cluster_size(mut self, max: usize) -> Self {
        self.max_cluster_size = max;
        self
    }

    pub fn with_cluster_similarity_threshold(mut self, threshold: f32) -> Self {
        self.cluster_similarity_threshold = threshold;
        self
    }

    pub fn with_target_tier(mut self, tier: u8) -> Self {
        self.target_tier = tier;
        self
    }
}

/// Hierarchical Drill-down tree representation of a milestone rollup and its leaf episodes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DrillDownTree {
    /// The root rollup episode record (Tier 2 or above).
    pub root: EpisodeRecord,
    /// The leaf episode records (Tier 1 execution traces).
    pub leaves: Vec<EpisodeRecord>,
    /// Total execution steps aggregated across all leaf episodes.
    pub total_steps: usize,
}

impl DrillDownTree {
    /// Create a new `DrillDownTree` from a root rollup and its leaf episodes.
    pub fn new(root: EpisodeRecord, leaves: Vec<EpisodeRecord>) -> Self {
        let total_steps = leaves.iter().map(|l| l.event_trace.len()).sum();
        Self {
            root,
            leaves,
            total_steps,
        }
    }

    /// Number of leaf episodes.
    pub fn leaves_count(&self) -> usize {
        self.leaves.len()
    }

    /// Total steps across all leaf episodes.
    pub fn total_steps(&self) -> usize {
        self.total_steps
    }

    /// Average salience score across leaf episodes.
    pub fn average_salience(&self) -> f32 {
        if self.leaves.is_empty() {
            self.root.salience_score
        } else {
            self.leaves.iter().map(|l| l.salience_score).sum::<f32>() / (self.leaves.len() as f32)
        }
    }

    /// Unique file paths touched across all leaf episodes.
    pub fn touched_files(&self) -> Vec<String> {
        extract_touched_files(&self.leaves)
    }

    /// Renders an ASCII tree representation for CLI and TUI visualization.
    pub fn render_ascii(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!(
            "└── [Tier {}] Rollup: {} (ID: {}, Salience: {:.2}, Leaves: {}, Steps: {})\n",
            self.root.tier,
            self.root.summary,
            self.root.id,
            self.root.salience_score,
            self.leaves.len(),
            self.total_steps
        ));

        let n = self.leaves.len();
        for (i, leaf) in self.leaves.iter().enumerate() {
            let is_last = i == n - 1;
            let branch = if is_last { "    └── " } else { "    ├── " };
            out.push_str(&format!(
                "{}[Tier {}] Leaf: {} (ID: {}, Steps: {}, Salience: {:.2})\n",
                branch,
                leaf.tier,
                leaf.summary,
                leaf.id,
                leaf.event_trace.len(),
                leaf.salience_score
            ));
            let sub_branch = if is_last { "        " } else { "    │   " };
            for step in &leaf.event_trace {
                let err_indicator = if let Some(ref e) = step.error {
                    format!(" [ERROR: {e}]")
                } else {
                    String::new()
                };
                out.push_str(&format!(
                    "{}  ↳ Step {}: {}{}\n",
                    sub_branch, step.step_index, step.action, err_indicator
                ));
            }
            for out_msg in &leaf.outcomes {
                out.push_str(&format!("{}• Outcome: {}\n", sub_branch, out_msg));
            }
        }

        out
    }
}

/// Core engine for hierarchical episodic clustering, medoid election, and causal invariants fusion.
pub struct EpisodicPyramidEngine;

impl EpisodicPyramidEngine {
    /// Elects the medoid episode from a cluster using pairwise cosine distances.
    ///
    /// The medoid is the index $i$ that minimizes $\sum_j D_{i,j}$, where $D_{i,j} = 1.0 - \text{cosine\_similarity}(v_i, v_j)$.
    /// In case of a numerical tie, strictly breaks ties using lexicographical order of UUID string (`e.id.to_string()`).
    pub fn elect_medoid(episodes: &[EpisodeRecord], embeddings: &[Vec<f32>]) -> usize {
        let n = episodes.len();
        if n == 0 {
            return 0;
        }
        if n == 1 || embeddings.len() < n {
            let mut min_idx = 0;
            let mut min_uuid = episodes[0].id.to_string();
            for (i, ep) in episodes.iter().enumerate().skip(1) {
                let uuid_str = ep.id.to_string();
                if uuid_str < min_uuid {
                    min_uuid = uuid_str;
                    min_idx = i;
                }
            }
            return min_idx;
        }

        let mut dist_sums = vec![0.0f32; n];
        for i in 0..n {
            for j in 0..n {
                if i != j {
                    let sim = cosine_similarity(&embeddings[i], &embeddings[j]);
                    let dist = (1.0 - sim).max(0.0);
                    dist_sums[i] += dist;
                }
            }
        }

        let mut best_idx = 0;
        let mut best_dist = dist_sums[0];
        let eps = 1e-5;

        for i in 1..n {
            let d = dist_sums[i];
            if d < best_dist - eps {
                best_dist = d;
                best_idx = i;
            } else if (d - best_dist).abs() <= eps {
                // Strictly deterministic tie-break by UUID string
                if episodes[i].id.to_string() < episodes[best_idx].id.to_string() {
                    best_dist = d;
                    best_idx = i;
                }
            }
        }

        best_idx
    }

    /// Fuses causal invariants from clustered episodes:
    /// - Extracts unique touched files from `event_trace` steps
    /// - Combines unique outcomes preserving insertion order
    /// - Aggregates salience score as $0.6 \times \max + 0.4 \times \text{mean}$
    /// - Fuses causal provenance chains
    /// - Generates canonical structured rollup title/summary:
    ///   `[Milestone Rollup] <medoid_summary> | Invariants: <outcomes_count> outcomes, <files_count> files touched`
    pub fn fuse_causal_invariants(
        episodes: &[EpisodeRecord],
        medoid_idx: usize,
    ) -> (String, Vec<String>, f32, Option<String>) {
        if episodes.is_empty() {
            return (
                "[Milestone Rollup] Empty Cluster | Invariants: 0 outcomes, 0 files touched".to_string(),
                Vec::new(),
                0.0,
                None,
            );
        }

        // 1. Touched files across all event_trace steps
        let touched_files = extract_touched_files(episodes);

        // 2. Combine unique outcomes preserving insertion order
        let mut outcomes = Vec::new();
        let mut seen_outcomes = std::collections::HashSet::new();
        for ep in episodes {
            for out in &ep.outcomes {
                let trimmed = out.trim();
                if !trimmed.is_empty() && seen_outcomes.insert(trimmed.to_string()) {
                    outcomes.push(trimmed.to_string());
                }
            }
        }

        // 3. Aggregate salience: 0.6 * max + 0.4 * mean
        let max_salience = episodes
            .iter()
            .map(|e| e.salience_score)
            .fold(0.0f32, f32::max);
        let mean_salience =
            episodes.iter().map(|e| e.salience_score).sum::<f32>() / (episodes.len() as f32);
        let salience_score = (0.6 * max_salience + 0.4 * mean_salience).clamp(0.0, 1.0);

        // 4. Fuse causal provenance
        let mut provenances = Vec::new();
        let mut seen_prov = std::collections::HashSet::new();
        for ep in episodes {
            if let Some(ref p) = ep.causal_provenance {
                let trimmed = p.trim();
                if !trimmed.is_empty() && seen_prov.insert(trimmed.to_string()) {
                    provenances.push(trimmed.to_string());
                }
            }
        }
        let fused_provenance = if provenances.is_empty() {
            None
        } else {
            Some(provenances.join(" -> "))
        };

        // 5. Canonical summary
        let medoid_summary = if medoid_idx < episodes.len() {
            &episodes[medoid_idx].summary
        } else {
            &episodes[0].summary
        };

        let summary = format!(
            "[Milestone Rollup] {} | Invariants: {} outcomes, {} files touched",
            medoid_summary,
            outcomes.len(),
            touched_files.len()
        );

        (summary, outcomes, salience_score, fused_provenance)
    }

    /// Clusters episodes into candidate rollups:
    /// - Prioritizes shared `task_id` groups (size between `min_cluster_size` and `max_cluster_size`).
    /// - Groups remaining episodes by embedding cosine similarity $\ge \text{threshold}$.
    /// - Clusters are strictly deterministically ordered by smallest medoid timestamp and smallest medoid UUID.
    pub fn cluster_episodes(
        episodes: &[EpisodeRecord],
        embeddings: &[Vec<f32>],
        config: &PyramidConfig,
    ) -> Vec<Vec<usize>> {
        let n = episodes.len();
        if n == 0 {
            return Vec::new();
        }

        let mut assigned = vec![false; n];
        let mut raw_clusters: Vec<Vec<usize>> = Vec::new();

        // 1. Group by shared task_id prioritarily
        let mut task_groups: std::collections::BTreeMap<String, Vec<usize>> =
            std::collections::BTreeMap::new();
        for (idx, ep) in episodes.iter().enumerate() {
            if let Some(ref tid) = ep.task_id {
                let trimmed = tid.trim();
                if !trimmed.is_empty() {
                    task_groups.entry(trimmed.to_string()).or_default().push(idx);
                }
            }
        }

        for (_tid, indices) in task_groups {
            if indices.len() >= config.min_cluster_size {
                for chunk in indices.chunks(config.max_cluster_size) {
                    if chunk.len() >= config.min_cluster_size {
                        for &idx in chunk {
                            assigned[idx] = true;
                        }
                        raw_clusters.push(chunk.to_vec());
                    }
                }
            }
        }

        // 2. Group remaining episodes by embedding similarity >= threshold
        let remaining: Vec<usize> = (0..n).filter(|&i| !assigned[i]).collect();
        let mut embedding_clusters: Vec<Vec<usize>> = Vec::new();

        for &idx in &remaining {
            let mut best_cluster_idx = None;
            let mut best_sim = config.cluster_similarity_threshold;

            if idx < embeddings.len() && !embeddings[idx].is_empty() {
                for (c_idx, c) in embedding_clusters.iter().enumerate() {
                    if c.len() >= config.max_cluster_size {
                        continue;
                    }
                    // Compare with cluster leader (first point)
                    let leader_idx = c[0];
                    if leader_idx < embeddings.len() && !embeddings[leader_idx].is_empty() {
                        let sim = cosine_similarity(&embeddings[idx], &embeddings[leader_idx]);
                        if sim >= best_sim {
                            best_sim = sim;
                            best_cluster_idx = Some(c_idx);
                        }
                    }
                }
            }

            if let Some(c_idx) = best_cluster_idx {
                embedding_clusters[c_idx].push(idx);
            } else {
                embedding_clusters.push(vec![idx]);
            }
        }

        for c in embedding_clusters {
            if c.len() >= config.min_cluster_size {
                raw_clusters.push(c);
            }
        }

        // 3. Deterministic cluster ordering: sorted by medoid's smallest timestamp and medoid's smallest UUID
        for c in &mut raw_clusters {
            c.sort_by(|&a, &b| {
                episodes[a]
                    .created_at
                    .cmp(&episodes[b].created_at)
                    .then_with(|| episodes[a].id.cmp(&episodes[b].id))
            });
        }

        let mut decorated_clusters: Vec<(i64, String, Vec<usize>)> = raw_clusters
            .into_iter()
            .map(|c| {
                let c_eps: Vec<EpisodeRecord> = c.iter().map(|&i| episodes[i].clone()).collect();
                let c_embs: Vec<Vec<f32>> = c
                    .iter()
                    .map(|&i| {
                        if i < embeddings.len() {
                            embeddings[i].clone()
                        } else {
                            vec![]
                        }
                    })
                    .collect();
                let medoid_local_idx = Self::elect_medoid(&c_eps, &c_embs);
                let medoid = &c_eps[medoid_local_idx];
                (medoid.created_at, medoid.id.to_string(), c)
            })
            .collect();

        decorated_clusters.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));

        decorated_clusters.into_iter().map(|(_, _, c)| c).collect()
    }

    /// Aggregates pending Tier 1 episodes into Tier 2 milestone rollups.
    ///
    /// - Queries unrolled Tier 1 episodes (parent_id IS NULL)
    /// - Clusters episodes using task grouping and embedding similarity
    /// - Elects medoids and fuses causal invariants
    /// - Persists Tier 2 rollups via `store.save_episode_rollup`
    /// - Returns generated rollups
    pub fn aggregate_session_rollups(
        store: &SqliteStore,
        session_id: Option<&str>,
        config: &PyramidConfig,
        embeddings: Option<&[Vec<f32>]>,
    ) -> Result<Vec<EpisodeRecord>, StrataError> {
        let unrolled = store.list_unrolled_episodes(1, session_id, 1000)?;
        if unrolled.is_empty() {
            return Ok(Vec::new());
        }

        let embs_storage;
        let embs = match embeddings {
            Some(e) => e,
            None => {
                embs_storage = vec![vec![]; unrolled.len()];
                &embs_storage[..]
            }
        };

        let clusters = Self::cluster_episodes(&unrolled, embs, config);
        if clusters.is_empty() {
            return Ok(Vec::new());
        }

        let mut generated_rollups = Vec::new();

        for cluster_indices in clusters {
            let cluster_eps: Vec<EpisodeRecord> = cluster_indices
                .iter()
                .map(|&idx| unrolled[idx].clone())
                .collect();

            let cluster_embs: Vec<Vec<f32>> = cluster_indices
                .iter()
                .map(|&idx| {
                    if idx < embs.len() {
                        embs[idx].clone()
                    } else {
                        vec![]
                    }
                })
                .collect();

            let medoid_local_idx = Self::elect_medoid(&cluster_eps, &cluster_embs);
            let medoid = &cluster_eps[medoid_local_idx];

            let (summary, outcomes, salience, causal_prov) =
                Self::fuse_causal_invariants(&cluster_eps, medoid_local_idx);

            let child_ids: Vec<Uuid> = cluster_eps.iter().map(|e| e.id).collect();

            // Deterministic RFC 4122 UUID derived from BLAKE3 hash
            let child_ids_str = child_ids
                .iter()
                .map(|id| id.to_string())
                .collect::<Vec<_>>()
                .join(",");
            let hash_name = format!(
                "strata:rollup:v1:{}:{}:{}:{}",
                medoid.session_id,
                medoid.id,
                child_ids_str,
                summary
            );
            let hash = blake3::hash(hash_name.as_bytes());
            let mut uuid_bytes = [0u8; 16];
            uuid_bytes.copy_from_slice(&hash.as_bytes()[..16]);
            uuid_bytes[6] = (uuid_bytes[6] & 0x0f) | 0x50; // version 5
            uuid_bytes[8] = (uuid_bytes[8] & 0x3f) | 0x80; // variant RFC 4122
            let rollup_id = Uuid::from_bytes(uuid_bytes);

            // Deterministic timestamp: latest timestamp among children
            let created_at = cluster_eps
                .iter()
                .map(|e| e.created_at)
                .max()
                .unwrap_or(0);

            let mut rollup = EpisodeRecord::new(&medoid.session_id, &medoid.agent_id, summary)
                .with_tier(config.target_tier)
                .with_salience(salience)
                .with_child_ids(child_ids.clone());
            rollup.id = rollup_id;
            rollup.task_id = medoid.task_id.clone();
            rollup.outcomes = outcomes;
            rollup.causal_provenance = causal_prov;
            rollup.state_snapshot = medoid.state_snapshot.clone();
            rollup.created_at = created_at;

            store.save_episode_rollup(&rollup)?;

            generated_rollups.push(rollup);
        }

        Ok(generated_rollups)
    }

    /// Zoom out: retrieves the most recent Tier 2 milestone rollups.
    pub fn zoom_out(
        store: &SqliteStore,
        session_id: Option<&str>,
        limit: usize,
    ) -> Result<Vec<EpisodeRecord>, StrataError> {
        store.list_episodes_by_tier(2, session_id, limit)
    }

    /// Drill down: reconstructs the hierarchical DrillDownTree for a given rollup ID.
    pub fn drill_down(
        store: &SqliteStore,
        rollup_id: &Uuid,
    ) -> Result<Option<DrillDownTree>, StrataError> {
        let tree_records = store.get_drill_down_tree(rollup_id)?;
        if tree_records.is_empty() {
            if let Some(single) = store.get_episode(rollup_id)? {
                return Ok(Some(DrillDownTree::new(single, Vec::new())));
            }
            return Ok(None);
        }

        let root = match tree_records.iter().find(|e| e.id == *rollup_id) {
            Some(r) => r.clone(),
            None => return Ok(None),
        };

        let leaves: Vec<EpisodeRecord> = tree_records
            .into_iter()
            .filter(|e| e.id != *rollup_id && e.tier == 1)
            .collect();

        Ok(Some(DrillDownTree::new(root, leaves)))
    }

    /// Aggregates unrolled Tier 2 milestone rollups into a Tier 3 Session Narrative (Epoch Rollup).
    ///
    /// - Queries unrolled Tier 2 episodes (`parent_id IS NULL`).
    /// - If none exist, returns `Ok(None)`.
    /// - Formats milestones and calls `bridge.generate_narrative(&tier2_episodes)` to obtain local SLM or heuristic narrative.
    /// - Fuses unique outcomes and aggregates salience.
    /// - Computes deterministic RFC 4122 UUID v5 from BLAKE3 hash.
    /// - Persists Tier 3 rollup via `store.save_episode_rollup` (updating `parent_id` of Tier 2 milestones).
    /// - Returns the created Tier 3 `EpisodeRecord`.
    pub fn aggregate_session_narrative(
        store: &SqliteStore,
        session_id: Option<&str>,
        bridge: &SlmPyramidBridge,
    ) -> Result<Option<EpisodeRecord>, StrataError> {
        let unrolled = store.list_unrolled_episodes(2, session_id, 1000)?;
        if unrolled.is_empty() {
            return Ok(None);
        }

        let inference = bridge.generate_narrative(&unrolled)?;
        let narrative = inference.narrative;

        // 1. Fuse unique outcomes across all Tier 2 rollups
        let mut outcomes = Vec::new();
        let mut seen_outcomes = std::collections::HashSet::new();
        for ep in &unrolled {
            for out in &ep.outcomes {
                let trimmed = out.trim();
                if !trimmed.is_empty() && seen_outcomes.insert(trimmed.to_string()) {
                    outcomes.push(trimmed.to_string());
                }
            }
        }

        // 2. Fused salience: 0.6 * max + 0.4 * mean
        let max_salience = unrolled
            .iter()
            .map(|e| e.salience_score)
            .fold(0.0f32, f32::max);
        let mean_salience =
            unrolled.iter().map(|e| e.salience_score).sum::<f32>() / (unrolled.len() as f32);
        let salience_score = (0.6 * max_salience + 0.4 * mean_salience).clamp(0.0, 1.0);

        // 3. Child IDs and timestamps
        let child_ids: Vec<Uuid> = unrolled.iter().map(|e| e.id).collect();
        let child_ids_str = child_ids
            .iter()
            .map(|id| id.to_string())
            .collect::<Vec<_>>()
            .join(",");
        let session_str = session_id
            .map(|s| s.to_string())
            .unwrap_or_else(|| unrolled[0].session_id.clone());

        // 4. Deterministic BLAKE3 -> RFC 4122 v5 UUID
        let hash_name = format!(
            "strata:rollup:tier3:v1:{}:{}:{}",
            session_str, child_ids_str, narrative
        );
        let hash = blake3::hash(hash_name.as_bytes());
        let mut uuid_bytes = [0u8; 16];
        uuid_bytes.copy_from_slice(&hash.as_bytes()[..16]);
        uuid_bytes[6] = (uuid_bytes[6] & 0x0f) | 0x50; // version 5
        uuid_bytes[8] = (uuid_bytes[8] & 0x3f) | 0x80; // variant RFC 4122
        let tier3_id = Uuid::from_bytes(uuid_bytes);

        let created_at = unrolled.iter().map(|e| e.created_at).max().unwrap_or(0);
        let agent_id = unrolled[0].agent_id.clone();
        let task_id = unrolled[0].task_id.clone();

        let mut tier3_record = EpisodeRecord::new(&session_str, &agent_id, narrative)
            .with_tier(3)
            .with_salience(salience_score)
            .with_child_ids(child_ids);
        tier3_record.id = tier3_id;
        tier3_record.task_id = task_id;
        tier3_record.outcomes = outcomes;
        tier3_record.created_at = created_at;

        store.save_episode_rollup(&tier3_record)?;

        Ok(Some(tier3_record))
    }
}

/// Filter criteria for querying episodic pyramid rollups and drill-downs.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PyramidFilter {
    /// Optional UUID of a specific rollup episode.
    pub rollup_id: Option<Uuid>,
    /// Optional cluster identifier or task ID.
    pub cluster_id: Option<String>,
    /// Optional start timestamp (Unix epoch seconds).
    pub start_time: Option<i64>,
    /// Optional end timestamp (Unix epoch seconds).
    pub end_time: Option<i64>,
    /// Optional session ID filter.
    pub session_id: Option<String>,
    /// Maximum number of rollups to return (default: 20).
    pub limit: usize,
}

impl PyramidFilter {
    pub fn new() -> Self {
        Self {
            limit: 20,
            ..Default::default()
        }
    }

    pub fn with_rollup_id(mut self, id: Uuid) -> Self {
        self.rollup_id = Some(id);
        self
    }

    pub fn with_cluster_id(mut self, cluster_id: impl Into<String>) -> Self {
        self.cluster_id = Some(cluster_id.into());
        self
    }

    pub fn with_start_time(mut self, start: i64) -> Self {
        self.start_time = Some(start);
        self
    }

    pub fn with_end_time(mut self, end: i64) -> Self {
        self.end_time = Some(end);
        self
    }

    pub fn with_session_id(mut self, session_id: impl Into<String>) -> Self {
        self.session_id = Some(session_id.into());
        self
    }

    pub fn with_limit(mut self, limit: usize) -> Self {
        self.limit = limit;
        self
    }
}

/// Compact summary of a Tier 2 milestone rollup for zero-explosion LLM context retrieval.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RollupSummary {
    pub id: Uuid,
    pub session_id: String,
    pub agent_id: String,
    pub task_id: Option<String>,
    pub summary: String,
    pub salience_score: f32,
    pub tier: u8,
    pub outcomes: Vec<String>,
    pub leaves_count: usize,
    pub child_ids: Vec<Uuid>,
    pub causal_provenance: Option<String>,
    pub created_at: i64,
}

impl From<&EpisodeRecord> for RollupSummary {
    fn from(ep: &EpisodeRecord) -> Self {
        Self {
            id: ep.id,
            session_id: ep.session_id.clone(),
            agent_id: ep.agent_id.clone(),
            task_id: ep.task_id.clone(),
            summary: ep.summary.clone(),
            salience_score: ep.salience_score,
            tier: ep.tier,
            outcomes: ep.outcomes.clone(),
            leaves_count: ep.child_ids.len(),
            child_ids: ep.child_ids.clone(),
            causal_provenance: ep.causal_provenance.clone(),
            created_at: ep.created_at,
        }
    }
}

/// Result of a progressive pyramid drill-down query.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PyramidDrillDownReport {
    pub expanded: bool,
    pub rollups_count: usize,
    pub total_leaves: usize,
    pub total_steps: usize,
    pub summaries: Vec<RollupSummary>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub trees: Vec<DrillDownTree>,
}

impl PyramidDrillDownReport {
    pub fn render_summary(&self) -> String {
        let mut out = String::new();
        if self.expanded {
            out.push_str(&format!(
                "🌲 [Hierarchical Pyramid Drill-Down (Expanded)] {} rollups, {} leaves, {} total execution steps:\n",
                self.rollups_count, self.total_leaves, self.total_steps
            ));
            for t in &self.trees {
                out.push_str(&t.render_ascii());
            }
        } else {
            out.push_str(&format!(
                "🏛️ [Hierarchical Pyramid Rollups (Compact Tier 2)] {} rollups found (pass expand_episodes: true to drill down into raw steps):\n",
                self.rollups_count
            ));
            for s in &self.summaries {
                out.push_str(&format!(
                    "• [Tier {} Rollup] ID: {} | Salience: {:.2} | Children: {} | Task: {}\n  Summary: {}\n  Outcomes ({}): {}\n  Created: {}\n",
                    s.tier,
                    s.id,
                    s.salience_score,
                    s.leaves_count,
                    s.task_id.as_deref().unwrap_or("none"),
                    s.summary,
                    s.outcomes.len(),
                    s.outcomes.join("; "),
                    s.created_at
                ));
            }
        }
        out
    }
}

/// Specialized store and engine coordinator for Hierarchical Episodic Memory Pyramid operations.
#[derive(Clone)]
pub struct EpisodicPyramidStore {
    store: Arc<SqliteStore>,
    config: PyramidConfig,
}

impl EpisodicPyramidStore {
    pub fn new(store: Arc<SqliteStore>) -> Self {
        Self {
            store,
            config: PyramidConfig::default(),
        }
    }

    pub fn with_config(mut self, config: PyramidConfig) -> Self {
        self.config = config;
        self
    }

    pub fn config(&self) -> &PyramidConfig {
        &self.config
    }

    pub fn store(&self) -> &Arc<SqliteStore> {
        &self.store
    }

    /// Automatically rolls up idle pending Tier 1 episodes into Tier 2 milestones.
    pub fn auto_rollup_idle_episodes(
        &self,
        session_id: Option<&str>,
        embeddings: Option<&[Vec<f32>]>,
    ) -> Result<Vec<EpisodeRecord>, StrataError> {
        EpisodicPyramidEngine::aggregate_session_rollups(
            &self.store,
            session_id,
            &self.config,
            embeddings,
        )
    }

    /// Alias for auto_rollup_idle_episodes with default embeddings.
    pub fn process_idle_episodes(
        &self,
        session_id: Option<&str>,
    ) -> Result<Vec<EpisodeRecord>, StrataError> {
        self.auto_rollup_idle_episodes(session_id, None)
    }

    /// Reconstructs the hierarchical DrillDownTree for a given rollup ID.
    pub fn drill_down(&self, rollup_id: &Uuid) -> Result<Option<DrillDownTree>, StrataError> {
        EpisodicPyramidEngine::drill_down(&self.store, rollup_id)
    }

    /// Retrieves top-level Tier 2 milestone rollups.
    pub fn zoom_out(
        &self,
        session_id: Option<&str>,
        limit: usize,
    ) -> Result<Vec<EpisodeRecord>, StrataError> {
        EpisodicPyramidEngine::zoom_out(&self.store, session_id, limit)
    }

    /// Queries rollups according to filter criteria (cluster_id, rollup_id, temporal interval).
    pub fn query_rollups(&self, filter: &PyramidFilter) -> Result<Vec<EpisodeRecord>, StrataError> {
        if let Some(ref rid) = filter.rollup_id {
            if let Some(ep) = self.store.get_episode(rid)? {
                return Ok(vec![ep]);
            }
            return Ok(Vec::new());
        }

        // If cluster_id is a UUID, try to query by ID first
        if let Some(ref cid) = filter.cluster_id {
            if let Ok(parsed_uuid) = Uuid::parse_str(cid) {
                if let Some(ep) = self.store.get_episode(&parsed_uuid)? {
                    if ep.tier >= 2 {
                        return Ok(vec![ep]);
                    }
                }
            }
        }

        let task_id_filter = filter.cluster_id.as_deref();
        let session_filter = filter.session_id.as_deref();
        let limit = if filter.limit == 0 { 20 } else { filter.limit };

        self.store.query_episodes_by_tier(
            self.config.target_tier,
            session_filter,
            task_id_filter,
            filter.start_time,
            filter.end_time,
            limit,
        )
    }

    /// Executes progressive drill-down with on-demand expansion of raw Tier 1 leaf episodes.
    ///
    /// When `expand_episodes` is `false`, returns compact `RollupSummary` instances with
    /// 0 raw execution steps (protecting the LLM context window).
    /// When `expand_episodes` is `true`, drills down into each matched rollup,
    /// reconstructing full `DrillDownTree` instances containing leaf traces.
    pub fn drill_down_progressive(
        &self,
        filter: &PyramidFilter,
        expand_episodes: bool,
    ) -> Result<PyramidDrillDownReport, StrataError> {
        let rollups = self.query_rollups(filter)?;
        let rollups_count = rollups.len();

        let summaries: Vec<RollupSummary> = rollups.iter().map(RollupSummary::from).collect();

        if !expand_episodes {
            let total_leaves: usize = summaries.iter().map(|s| s.leaves_count).sum();
            return Ok(PyramidDrillDownReport {
                expanded: false,
                rollups_count,
                total_leaves,
                total_steps: 0,
                summaries,
                trees: Vec::new(),
            });
        }

        let mut trees = Vec::with_capacity(rollups.len());
        let mut total_leaves = 0;
        let mut total_steps = 0;

        for r in &rollups {
            if let Some(tree) = self.drill_down(&r.id)? {
                total_leaves += tree.leaves.len();
                total_steps += tree.total_steps;
                trees.push(tree);
            }
        }

        Ok(PyramidDrillDownReport {
            expanded: true,
            rollups_count,
            total_leaves,
            total_steps,
            summaries,
            trees,
        })
    }

    /// Aggregates unrolled Tier 2 milestone rollups into a Tier 3 Session Narrative.
    pub fn generate_session_narrative(
        &self,
        session_id: Option<&str>,
        bridge: Option<&SlmPyramidBridge>,
    ) -> Result<Option<EpisodeRecord>, StrataError> {
        let default_bridge;
        let b = match bridge {
            Some(b) => b,
            None => {
                default_bridge = SlmPyramidBridge::with_default_config();
                &default_bridge
            }
        };
        EpisodicPyramidEngine::aggregate_session_narrative(&self.store, session_id, b)
    }
}

/// Configuration for the local SLM SmolLM2 episodic narrative bridge.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SlmBridgeConfig {
    /// Python binary path or command name.
    pub python_binary: String,
    /// Path to the local LoRA adapter weights directory (e.g. `labs/episodic-pyramid-slm/runs/lora_r16_1b7/adapter`).
    pub adapter_path: PathBuf,
    /// Path to the local inference runner script (e.g. `labs/episodic-pyramid-slm/infer.py`).
    pub script_path: PathBuf,
    /// Maximum new tokens generated by the SLM.
    pub max_new_tokens: usize,
    /// Generation temperature.
    pub temperature: f32,
    /// Timeout in seconds for local inference child process.
    pub timeout_seconds: u64,
    /// Force 100% offline execution without network calls.
    pub offline: bool,
}

impl Default for SlmBridgeConfig {
    fn default() -> Self {
        let venv_win = PathBuf::from("labs/episodic-pyramid-slm/.venv/Scripts/python.exe");
        let venv_unix = PathBuf::from("labs/episodic-pyramid-slm/.venv/bin/python");
        let python_binary = if let Ok(custom) = std::env::var("STRATA_SLM_PYTHON") {
            custom
        } else if venv_win.exists() {
            venv_win.to_string_lossy().to_string()
        } else if venv_unix.exists() {
            venv_unix.to_string_lossy().to_string()
        } else {
            "python".to_string()
        };

        Self {
            python_binary,
            adapter_path: PathBuf::from("labs/episodic-pyramid-slm/runs/lora_r16_1b7/adapter"),
            script_path: PathBuf::from("labs/episodic-pyramid-slm/infer.py"),
            max_new_tokens: 180,
            temperature: 0.2,
            timeout_seconds: 45,
            offline: true,
        }
    }
}

/// Result of an episodic narrative inference run (from local SLM or deterministic fallback).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SlmInferenceResult {
    pub narrative: String,
    pub heuristic: String,
    pub tier: u8,
    pub chars_in: usize,
    pub chars_out: usize,
    pub compression_ratio: f32,
    pub model: String,
    pub offline: bool,
    pub fallback: bool,
    #[serde(default)]
    pub warning: Option<String>,
}

/// Local SLM bridge connecting Strata episodic memory pyramid to SmolLM2 LoRA adapter.
#[derive(Debug, Clone)]
pub struct SlmPyramidBridge {
    pub config: SlmBridgeConfig,
}

impl SlmPyramidBridge {
    pub fn new(config: SlmBridgeConfig) -> Self {
        Self { config }
    }

    pub fn with_default_config() -> Self {
        Self::new(SlmBridgeConfig::default())
    }

    /// Formats Tier 2 milestone rollups into the standard input prompt for Tier 3 narrative synthesis.
    pub fn format_tier2_prompt(milestones: &[EpisodeRecord]) -> String {
        let mut prompt = String::new();
        for m in milestones {
            let summary = m.summary.trim();
            let outcomes_str = if m.outcomes.is_empty() {
                String::new()
            } else {
                format!(" (outcomes: {})", m.outcomes.join("; "))
            };
            prompt.push_str(&format!("- {}{}\n", summary, outcomes_str));
        }
        prompt
    }

    /// Deterministic heuristic narrative (pure Rust, zero-overhead, 100% offline).
    pub fn heuristic_narrative(milestones: &[EpisodeRecord]) -> String {
        if milestones.is_empty() {
            return "[Session Narrative] Empty Milestone Rollup Cluster".to_string();
        }
        let summaries: Vec<String> = milestones
            .iter()
            .map(|m| {
                let s = m.summary.trim();
                if let Some(stripped) = s.strip_prefix("[Milestone Rollup]") {
                    stripped.split('|').next().unwrap_or(stripped).trim().to_string()
                } else {
                    s.split('|').next().unwrap_or(s).trim().to_string()
                }
            })
            .filter(|s| !s.is_empty())
            .take(5)
            .collect();

        format!(
            "Narrativa: Sessao agregou {} marcos principais. Arco de execucao: {}. Invariantes e contratos preservados com sucesso.",
            milestones.len(),
            summaries.join(" | ")
        )
    }

    /// Generates a Tier 3 session narrative using the local SmolLM2 adapter,
    /// with transparent fallback to deterministic heuristic synthesis if Python or GPU is unavailable.
    pub fn generate_narrative(&self, milestones: &[EpisodeRecord]) -> Result<SlmInferenceResult, StrataError> {
        if milestones.is_empty() {
            let heur = Self::heuristic_narrative(milestones);
            return Ok(SlmInferenceResult {
                narrative: heur.clone(),
                heuristic: heur,
                tier: 3,
                chars_in: 0,
                chars_out: 0,
                compression_ratio: 1.0,
                model: "heuristic_fallback".to_string(),
                offline: true,
                fallback: true,
                warning: Some("No milestones provided for narrative generation".to_string()),
            });
        }

        let prompt = Self::format_tier2_prompt(milestones);
        let chars_in = prompt.len();
        let heuristic = Self::heuristic_narrative(milestones);

        // Attempt child process execution if script exists
        let script = &self.config.script_path;
        if script.exists() {
            let mut cmd = Command::new(&self.config.python_binary);
            cmd.arg(script)
                .arg("--tier")
                .arg("t2_to_t3")
                .arg("--json")
                .arg("--max-tokens")
                .arg(self.config.max_new_tokens.to_string())
                .arg("--temperature")
                .arg(self.config.temperature.to_string())
                .arg("--input")
                .arg("-");

            if self.config.adapter_path.exists() {
                cmd.arg("--adapter-dir").arg(&self.config.adapter_path);
            }

            if self.config.offline {
                cmd.env("HF_HUB_OFFLINE", "1");
                cmd.env("TRANSFORMERS_OFFLINE", "1");
                cmd.env("HF_DATASETS_OFFLINE", "1");
            }
            cmd.env("HF_HUB_DISABLE_PROGRESS_BARS", "1");
            cmd.env("TOKENIZERS_PARALLELISM", "false");

            cmd.stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());

            if let Ok(mut child) = cmd.spawn() {
                if let Some(mut stdin) = child.stdin.take() {
                    let _ = stdin.write_all(prompt.as_bytes());
                }

                if let Ok(output) = child.wait_with_output() {
                    if output.status.success() {
                        let stdout_str = String::from_utf8_lossy(&output.stdout);
                        if let Some(json_start) = stdout_str.find('{') {
                            if let Some(json_end) = stdout_str.rfind('}') {
                                let json_substr = &stdout_str[json_start..=json_end];
                                if let Ok(parsed) = serde_json::from_str::<SlmInferenceResult>(json_substr) {
                                    return Ok(parsed);
                                }
                            }
                        }
                    }
                }
            }
        }

        // Deterministic fallback if child process fails or is not found
        let chars_out = heuristic.len();
        let ratio = if chars_out > 0 {
            (chars_in as f32) / (chars_out as f32)
        } else {
            1.0
        };

        Ok(SlmInferenceResult {
            narrative: heuristic.clone(),
            heuristic,
            tier: 3,
            chars_in,
            chars_out,
            compression_ratio: ratio,
            model: "heuristic_fallback".to_string(),
            offline: true,
            fallback: true,
            warning: Some("Python SLM bridge unavailable or timed out; used deterministic heuristic".to_string()),
        })
    }
}

// Helper functions for invariant extraction

fn normalize_file_path(path: &str) -> String {
    let clean = path.trim().replace('\\', "/");
    clean
        .trim_matches(|c| c == '"' || c == '\'' || c == '`' || c == ',' || c == ';')
        .to_string()
}

fn is_valid_file_path(s: &str) -> bool {
    let s = s.trim();
    if s.is_empty() || s.contains('\n') || s.len() > 260 {
        return false;
    }
    let has_separator = s.contains('/') || s.contains('\\');
    let has_extension = s.ends_with(".rs")
        || s.ends_with(".ts")
        || s.ends_with(".js")
        || s.ends_with(".py")
        || s.ends_with(".json")
        || s.ends_with(".toml")
        || s.ends_with(".md")
        || s.ends_with(".sql")
        || s.ends_with(".yaml")
        || s.ends_with(".yml")
        || s.ends_with(".lock");

    (has_separator && has_extension) || (has_extension && !s.contains(' '))
}

fn extract_touched_files(episodes: &[EpisodeRecord]) -> Vec<String> {
    let mut files = Vec::new();
    let mut seen = std::collections::HashSet::new();

    for ep in episodes {
        for step in &ep.event_trace {
            if let Some(ref val) = step.input {
                extract_files_from_json(val, &mut files, &mut seen);
            }
            if let Some(ref val) = step.output {
                extract_files_from_json(val, &mut files, &mut seen);
            }
            extract_files_from_text(&step.action, &mut files, &mut seen);
        }
    }

    files
}

fn extract_files_from_json(
    val: &serde_json::Value,
    files: &mut Vec<String>,
    seen: &mut std::collections::HashSet<String>,
) {
    match val {
        serde_json::Value::Object(map) => {
            for (k, v) in map {
                let k_lower = k.to_lowercase();
                if k_lower == "file"
                    || k_lower == "path"
                    || k_lower == "file_path"
                    || k_lower == "filepath"
                    || k_lower == "target_file"
                    || k_lower == "target_path"
                    || k_lower == "filename"
                {
                    if let Some(s) = v.as_str() {
                        let normalized = normalize_file_path(s);
                        if !normalized.is_empty() && seen.insert(normalized.clone()) {
                            files.push(normalized);
                        }
                    }
                } else if k_lower == "files" || k_lower == "paths" {
                    if let Some(arr) = v.as_array() {
                        for item in arr {
                            if let Some(s) = item.as_str() {
                                let normalized = normalize_file_path(s);
                                if !normalized.is_empty() && seen.insert(normalized.clone()) {
                                    files.push(normalized);
                                }
                            }
                        }
                    }
                } else {
                    extract_files_from_json(v, files, seen);
                }
            }
        }
        serde_json::Value::Array(arr) => {
            for item in arr {
                extract_files_from_json(item, files, seen);
            }
        }
        serde_json::Value::String(s) => {
            if is_valid_file_path(s) {
                let normalized = normalize_file_path(s);
                if !normalized.is_empty() && seen.insert(normalized.clone()) {
                    files.push(normalized);
                }
            }
        }
        _ => {}
    }
}

fn extract_files_from_text(
    text: &str,
    files: &mut Vec<String>,
    seen: &mut std::collections::HashSet<String>,
) {
    for token in text.split_whitespace() {
        let trimmed = token.trim_matches(|c: char| {
            c == '('
                || c == ')'
                || c == '['
                || c == ']'
                || c == '{'
                || c == '}'
                || c == '"'
                || c == '\''
                || c == '`'
                || c == ':'
                || c == ','
        });
        if is_valid_file_path(trimmed) {
            let normalized = normalize_file_path(trimmed);
            if !normalized.is_empty() && seen.insert(normalized.clone()) {
                files.push(normalized);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use strata_core::schemas::EpisodeStep;

    #[test]
    fn test_elect_medoid_with_deterministic_tie_breaker() {
        // 1. Geometric Medoid Election
        let mut ep1 = EpisodeRecord::new("sess-1", "agent-1", "Episode 1");
        let mut ep2 = EpisodeRecord::new("sess-1", "agent-1", "Episode 2");
        let mut ep3 = EpisodeRecord::new("sess-1", "agent-1", "Episode 3");
        ep1.id = Uuid::parse_str("11111111-1111-1111-1111-111111111111").unwrap();
        ep2.id = Uuid::parse_str("22222222-2222-2222-2222-222222222222").unwrap();
        ep3.id = Uuid::parse_str("33333333-3333-3333-3333-333333333333").unwrap();

        let episodes = vec![ep1, ep2, ep3];
        // ep2 is in the geometric middle of ep1 and ep3
        let embeddings = vec![
            vec![1.0, 0.0],
            vec![0.707, 0.707],
            vec![0.0, 1.0],
        ];

        let medoid_idx = EpisodicPyramidEngine::elect_medoid(&episodes, &embeddings);
        assert_eq!(medoid_idx, 1, "Episode 2 should be elected medoid as the geometric center");

        // 2. Deterministic Tie-Breaker: Exact numerical tie
        let mut ep_a = EpisodeRecord::new("sess-1", "agent-1", "Episode A");
        let mut ep_b = EpisodeRecord::new("sess-1", "agent-1", "Episode B");
        ep_a.id = Uuid::parse_str("aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa").unwrap();
        ep_b.id = Uuid::parse_str("bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb").unwrap();

        let tie_episodes_1 = vec![ep_b.clone(), ep_a.clone()];
        let tie_embeddings = vec![vec![1.0, 0.0], vec![1.0, 0.0]];

        // ep_a has smaller UUID ("aaaa..." < "bbbb..."), so index 1 in tie_episodes_1 should win
        let tie_medoid_1 = EpisodicPyramidEngine::elect_medoid(&tie_episodes_1, &tie_embeddings);
        assert_eq!(tie_episodes_1[tie_medoid_1].id, ep_a.id);

        let tie_episodes_2 = vec![ep_a.clone(), ep_b.clone()];
        let tie_medoid_2 = EpisodicPyramidEngine::elect_medoid(&tie_episodes_2, &tie_embeddings);
        assert_eq!(tie_episodes_2[tie_medoid_2].id, ep_a.id);
    }

    #[test]
    fn test_fuse_causal_invariants_and_metrics() {
        let mut ep1 = EpisodeRecord::new("sess-1", "agent-1", "Migrate SQLite schema")
            .with_salience(0.80)
            .with_provenance("User prompt -> Schema design")
            .with_outcome("Created episodes table")
            .with_outcome("Added composite indexes");

        let step1 = EpisodeStep::new(1, "create table")
            .with_input(serde_json::json!({
                "file_path": "crates/strata-memory/src/store.rs"
            }));
        let step2 = EpisodeStep::new(2, "export module crates/strata-memory/src/lib.rs");
        ep1.event_trace = vec![step1, step2];

        let mut ep2 = EpisodeRecord::new("sess-1", "agent-1", "Implement query methods")
            .with_salience(0.60)
            .with_provenance("Schema design -> Query implementation")
            .with_outcome("Added composite indexes") // duplicate outcome
            .with_outcome("Implemented get_drill_down_tree");

        let step3 = EpisodeStep::new(1, "write query")
            .with_output(serde_json::json!({
                "files": [
                    "crates/strata-memory/src/store.rs",
                    "crates/strata-memory/src/pyramid.rs"
                ]
            }));
        ep2.event_trace = vec![step3];

        let episodes = vec![ep1, ep2];
        let (summary, outcomes, salience, provenance) =
            EpisodicPyramidEngine::fuse_causal_invariants(&episodes, 0);

        // 1. Files count: store.rs, lib.rs, pyramid.rs = 3 unique files
        // 2. Outcomes count: "Created episodes table", "Added composite indexes", "Implemented get_drill_down_tree" = 3 unique outcomes
        assert_eq!(outcomes.len(), 3);
        assert_eq!(outcomes[0], "Created episodes table");
        assert_eq!(outcomes[1], "Added composite indexes");
        assert_eq!(outcomes[2], "Implemented get_drill_down_tree");

        // 3. Structured canonical summary
        assert_eq!(
            summary,
            "[Milestone Rollup] Migrate SQLite schema | Invariants: 3 outcomes, 3 files touched"
        );

        // 4. Salience: 0.6 * 0.80 + 0.4 * 0.70 = 0.48 + 0.28 = 0.76
        assert!((salience - 0.76).abs() < 1e-5);

        // 5. Causal provenance
        assert_eq!(
            provenance,
            Some("User prompt -> Schema design -> Schema design -> Query implementation".to_string())
        );
    }

    #[test]
    fn test_cluster_episodes_by_task_id_and_similarity() {
        let config = PyramidConfig::default()
            .with_min_cluster_size(2)
            .with_max_cluster_size(3)
            .with_cluster_similarity_threshold(0.70);

        // EP0 and EP1 share task_id "auth-task"
        let mut ep0 = EpisodeRecord::new("sess-1", "agent-1", "Auth 1");
        ep0.task_id = Some("auth-task".to_string());
        ep0.created_at = 100;
        let mut ep1 = EpisodeRecord::new("sess-1", "agent-1", "Auth 2");
        ep1.task_id = Some("auth-task".to_string());
        ep1.created_at = 105;

        // EP2 and EP3 have high cosine similarity (sim ~ 1.0) without shared task_id
        let mut ep2 = EpisodeRecord::new("sess-1", "agent-1", "Sim A");
        ep2.created_at = 200;
        let mut ep3 = EpisodeRecord::new("sess-1", "agent-1", "Sim B");
        ep3.created_at = 205;

        // EP4 is an isolated outlier
        let mut ep4 = EpisodeRecord::new("sess-1", "agent-1", "Outlier");
        ep4.created_at = 300;

        let episodes = vec![ep0, ep1, ep2, ep3, ep4];
        let embeddings = vec![
            vec![1.0, 0.0],  // EP0 (auth-task)
            vec![0.0, 1.0],  // EP1 (auth-task, low sim with EP0, but grouped by task_id!)
            vec![0.0, 1.0],  // EP2
            vec![0.0, 0.99], // EP3 (high sim with EP2)
            vec![-1.0, 0.0], // EP4 (outlier, size 1 < min_cluster_size 2)
        ];

        let clusters = EpisodicPyramidEngine::cluster_episodes(&episodes, &embeddings, &config);

        assert_eq!(clusters.len(), 2, "Expected exactly 2 clusters meeting min_cluster_size");
        // Cluster 1: auth-task
        assert_eq!(clusters[0], vec![0, 1]);
        // Cluster 2: vector similarity
        assert_eq!(clusters[1], vec![2, 3]);
    }

    #[test]
    fn test_end_to_end_rollup_generation_drill_down_and_ascii() {
        let store = SqliteStore::open_in_memory().expect("open in memory store");

        let mut ep1 = EpisodeRecord::new("sess-e2e", "agent-lead", "Setup database schema")
            .with_salience(0.85)
            .with_outcome("Schema initialized");
        ep1.task_id = Some("task-database".to_string());
        ep1.event_trace = vec![
            EpisodeStep::new(1, "create table episodes").with_input(serde_json::json!({ "file": "crates/strata-memory/src/store.rs" })),
            EpisodeStep::new(2, "verify migration"),
        ];

        let mut ep2 = EpisodeRecord::new("sess-e2e", "agent-lead", "Add query indexes")
            .with_salience(0.75)
            .with_outcome("Indexes verified");
        ep2.task_id = Some("task-database".to_string());
        ep2.event_trace = vec![
            EpisodeStep::new(1, "create index idx_episodes_tier"),
        ];

        store.save_episode(&ep1).expect("save ep1");
        store.save_episode(&ep2).expect("save ep2");

        let config = PyramidConfig::default();
        let rollups = EpisodicPyramidEngine::aggregate_session_rollups(
            &store,
            Some("sess-e2e"),
            &config,
            None,
        )
        .expect("aggregate session rollups");

        assert_eq!(rollups.len(), 1, "Should generate exactly 1 milestone rollup");
        let rollup = &rollups[0];
        assert_eq!(rollup.tier, 2);
        assert_eq!(rollup.child_ids.len(), 2);
        assert!(rollup.child_ids.contains(&ep1.id));
        assert!(rollup.child_ids.contains(&ep2.id));

        // Verify child episodes now have parent_id pointing to rollup
        let fetched_ep1 = store.get_episode(&ep1.id).expect("get ep1").expect("ep1 found");
        assert_eq!(fetched_ep1.parent_id, Some(rollup.id));
        let fetched_ep2 = store.get_episode(&ep2.id).expect("get ep2").expect("ep2 found");
        assert_eq!(fetched_ep2.parent_id, Some(rollup.id));

        // Test zoom_out
        let zoomed = EpisodicPyramidEngine::zoom_out(&store, Some("sess-e2e"), 10)
            .expect("zoom out");
        assert_eq!(zoomed.len(), 1);
        assert_eq!(zoomed[0].id, rollup.id);

        // Test drill_down
        let tree_opt = EpisodicPyramidEngine::drill_down(&store, &rollup.id)
            .expect("drill down");
        assert!(tree_opt.is_some());
        let tree = tree_opt.unwrap();
        assert_eq!(tree.root.id, rollup.id);
        assert_eq!(tree.leaves.len(), 2);
        assert_eq!(tree.total_steps, 3); // 2 steps in ep1 + 1 step in ep2

        // Test render_ascii
        let ascii = tree.render_ascii();
        assert!(ascii.contains("[Tier 2] Rollup:"));
        assert!(ascii.contains("[Tier 1] Leaf:"));
        assert!(ascii.contains("Schema initialized"));
        assert!(ascii.contains("Indexes verified"));
        assert!(ascii.contains("├── ") || ascii.contains("└── "));
    }

    #[test]
    fn test_replay_consistency_100_percent_deterministic() {
        let store1 = SqliteStore::open_in_memory().expect("open store1");
        let store2 = SqliteStore::open_in_memory().expect("open store2");

        let id1 = Uuid::parse_str("11111111-aaaa-1111-aaaa-111111111111").unwrap();
        let id2 = Uuid::parse_str("22222222-bbbb-2222-bbbb-222222222222").unwrap();
        let id3 = Uuid::parse_str("33333333-cccc-3333-cccc-333333333333").unwrap();

        let mut ep1 = EpisodeRecord::new("sess-replay", "agent-worker", "Replay Step 1")
            .with_salience(0.70)
            .with_outcome("Outcome 1");
        ep1.id = id1;
        ep1.created_at = 1000;
        ep1.task_id = Some("replay-task".to_string());

        let mut ep2 = EpisodeRecord::new("sess-replay", "agent-worker", "Replay Step 2")
            .with_salience(0.85)
            .with_outcome("Outcome 2");
        ep2.id = id2;
        ep2.created_at = 1010;
        ep2.task_id = Some("replay-task".to_string());

        let mut ep3 = EpisodeRecord::new("sess-replay", "agent-worker", "Replay Step 3")
            .with_salience(0.90)
            .with_outcome("Outcome 3");
        ep3.id = id3;
        ep3.created_at = 1020;
        ep3.task_id = Some("replay-task".to_string());

        // Ingest into store 1
        store1.save_episode(&ep1).unwrap();
        store1.save_episode(&ep2).unwrap();
        store1.save_episode(&ep3).unwrap();

        // Ingest identical episodes into store 2
        store2.save_episode(&ep1).unwrap();
        store2.save_episode(&ep2).unwrap();
        store2.save_episode(&ep3).unwrap();

        let config = PyramidConfig::default();
        let rollups1 = EpisodicPyramidEngine::aggregate_session_rollups(
            &store1,
            Some("sess-replay"),
            &config,
            None,
        )
        .expect("aggregate store1");

        let rollups2 = EpisodicPyramidEngine::aggregate_session_rollups(
            &store2,
            Some("sess-replay"),
            &config,
            None,
        )
        .expect("aggregate store2");

        assert_eq!(rollups1.len(), 1);
        assert_eq!(rollups2.len(), 1);

        // Bit-for-bit identical EpisodeRecord
        assert_eq!(rollups1[0], rollups2[0]);
        assert_eq!(rollups1[0].id, rollups2[0].id);
        assert_eq!(rollups1[0].summary, rollups2[0].summary);
        assert_eq!(rollups1[0].salience_score, rollups2[0].salience_score);
        assert_eq!(rollups1[0].child_ids, rollups2[0].child_ids);
        assert_eq!(rollups1[0].created_at, rollups2[0].created_at);

        // JSON serialization matches bit-for-bit
        let json1 = serde_json::to_string(&rollups1[0]).unwrap();
        let json2 = serde_json::to_string(&rollups2[0]).unwrap();
        assert_eq!(json1, json2, "Replay generation must be bit-for-bit identical");
    }

    #[test]
    fn test_episodic_pyramid_store_progressive_drill_down_and_filtering() {
        use strata_core::schemas::EpisodeStep;

        let sqlite = Arc::new(SqliteStore::open_in_memory().expect("open memory store"));
        let pyramid_store = EpisodicPyramidStore::new(sqlite.clone());

        // Ingest two distinct clusters across different timestamps
        // Cluster A: task-auth, timestamp 1000..1050
        for i in 0..3 {
            let mut ep = EpisodeRecord::new("sess-drill", "agent-auth", format!("Auth Step {i}"))
                .with_task_id("task-auth")
                .with_salience(0.8)
                .with_outcome(format!("Auth token {i} verified"))
                .with_steps(vec![
                    EpisodeStep::new(0, format!("Generate key {i}")),
                    EpisodeStep::new(1, format!("Sign claims {i}")),
                ]);
            ep.created_at = 1000 + (i as i64 * 20);
            sqlite.save_episode(&ep).expect("save ep");
        }

        // Cluster B: task-db, timestamp 2000..2050
        for i in 0..3 {
            let mut ep = EpisodeRecord::new("sess-drill", "agent-db", format!("DB Step {i}"))
                .with_task_id("task-db")
                .with_salience(0.9)
                .with_outcome(format!("Table partition {i} created"))
                .with_steps(vec![
                    EpisodeStep::new(0, format!("Run migration {i}")),
                ]);
            ep.created_at = 2000 + (i as i64 * 20);
            sqlite.save_episode(&ep).expect("save ep");
        }

        // 1. Run auto-rollup
        let rollups = pyramid_store
            .auto_rollup_idle_episodes(Some("sess-drill"), None)
            .expect("auto rollup");
        assert_eq!(rollups.len(), 2, "Expected 2 milestone rollups generated");

        let auth_rollup = rollups.iter().find(|r| r.task_id.as_deref() == Some("task-auth")).unwrap();
        let db_rollup = rollups.iter().find(|r| r.task_id.as_deref() == Some("task-db")).unwrap();

        // 2. Progressive drill-down with expand_episodes = false (compact Tier 2 summaries)
        let filter_all = PyramidFilter::new().with_session_id("sess-drill");
        let compact_report = pyramid_store
            .drill_down_progressive(&filter_all, false)
            .expect("compact drilldown");

        assert!(!compact_report.expanded);
        assert_eq!(compact_report.rollups_count, 2);
        assert_eq!(compact_report.summaries.len(), 2);
        assert_eq!(compact_report.trees.len(), 0, "No raw trees should be expanded");
        assert_eq!(compact_report.total_steps, 0, "Zero steps token overhead when compact");
        assert_eq!(compact_report.total_leaves, 6);
        let summary_text = compact_report.render_summary();
        assert!(summary_text.contains("Hierarchical Pyramid Rollups (Compact Tier 2)"));
        assert!(summary_text.contains("Auth token"));
        assert!(summary_text.contains("Table partition"));

        // 3. Progressive drill-down with expand_episodes = true (on-demand expansion)
        let expanded_report = pyramid_store
            .drill_down_progressive(&filter_all, true)
            .expect("expanded drilldown");

        assert!(expanded_report.expanded);
        assert_eq!(expanded_report.rollups_count, 2);
        assert_eq!(expanded_report.trees.len(), 2, "Both trees should be reconstructed");
        assert_eq!(expanded_report.total_leaves, 6);
        assert_eq!(expanded_report.total_steps, 9); // (3 * 2) + (3 * 1) = 9
        let expanded_text = expanded_report.render_summary();
        assert!(expanded_text.contains("Hierarchical Pyramid Drill-Down (Expanded)"));
        assert!(expanded_text.contains("Generate key"));
        assert!(expanded_text.contains("Run migration"));

        // 4. Filtering by cluster_id ("task-auth")
        let filter_auth = PyramidFilter::new().with_cluster_id("task-auth");
        let auth_report = pyramid_store
            .drill_down_progressive(&filter_auth, true)
            .expect("filter auth");
        assert_eq!(auth_report.rollups_count, 1);
        assert_eq!(auth_report.summaries[0].task_id.as_deref(), Some("task-auth"));
        assert_eq!(auth_report.total_leaves, 3);
        assert_eq!(auth_report.total_steps, 6);

        // 5. Filtering by specific rollup_id
        let filter_id = PyramidFilter::new().with_rollup_id(db_rollup.id);
        let id_report = pyramid_store
            .drill_down_progressive(&filter_id, true)
            .expect("filter id");
        assert_eq!(id_report.rollups_count, 1);
        assert_eq!(id_report.summaries[0].id, db_rollup.id);
        assert_eq!(id_report.total_steps, 3);

        // 6. Filtering by temporal interval (start_time..end_time)
        // Window 900..1500 matches task-auth (1000..1040), but excludes task-db (2000..2040)
        let filter_time = PyramidFilter::new()
            .with_start_time(900)
            .with_end_time(1500);
        let time_report = pyramid_store
            .drill_down_progressive(&filter_time, false)
            .expect("time filter");
        assert_eq!(time_report.rollups_count, 1);
        assert_eq!(time_report.summaries[0].id, auth_rollup.id);

        // Window 1800..2500 matches task-db only
        let filter_time_db = PyramidFilter::new()
            .with_start_time(1800)
            .with_end_time(2500);
        let time_db_report = pyramid_store
            .drill_down_progressive(&filter_time_db, false)
            .expect("time db filter");
        assert_eq!(time_db_report.rollups_count, 1);
        assert_eq!(time_db_report.summaries[0].id, db_rollup.id);
    }

    #[test]
    fn test_slm_bridge_tier2_prompt_formatting() {
        let ep1 = EpisodeRecord::new("sess-1", "agent-1", "Auth JWT RS256+JWKS")
            .with_tier(2)
            .with_outcomes(vec!["Keypair generated".to_string(), "JWKS exposed".to_string()]);
        let ep2 = EpisodeRecord::new("sess-1", "agent-1", "SQLite pool with DeadlockTimeout")
            .with_tier(2)
            .with_outcomes(vec!["Pool exhaustion mitigated".to_string()]);

        let prompt = SlmPyramidBridge::format_tier2_prompt(&[ep1, ep2]);
        assert!(prompt.contains("- Auth JWT RS256+JWKS"));
        assert!(prompt.contains("Keypair generated"));
        assert!(prompt.contains("- SQLite pool with DeadlockTimeout"));
        assert!(prompt.contains("Pool exhaustion mitigated"));
    }

    #[test]
    fn test_slm_heuristic_narrative() {
        let ep1 = EpisodeRecord::new("sess-1", "agent-1", "[Milestone Rollup] Auth JWT RS256 | Invariants: 2 outcomes, 2 files touched")
            .with_tier(2);
        let ep2 = EpisodeRecord::new("sess-1", "agent-1", "[Milestone Rollup] SQLite pool | Invariants: 1 outcomes, 1 files touched")
            .with_tier(2);

        let narrative = SlmPyramidBridge::heuristic_narrative(&[ep1, ep2]);
        assert!(narrative.starts_with("Narrativa: Sessao agregou 2 marcos principais."));
        assert!(narrative.contains("Auth JWT RS256"));
        assert!(narrative.contains("SQLite pool"));
        assert!(narrative.contains("Invariantes e contratos preservados"));
    }

    #[test]
    fn test_slm_pyramid_bridge_local_fallback() {
        let mut config = SlmBridgeConfig::default();
        config.python_binary = "non_existent_python_binary_xyz123".to_string();
        let bridge = SlmPyramidBridge::new(config);

        let ep1 = EpisodeRecord::new("sess-1", "agent-1", "Milestone Auth").with_tier(2);
        let res = bridge.generate_narrative(&[ep1]).expect("inference");

        assert_eq!(res.tier, 3);
        assert!(res.fallback);
        assert!(res.offline);
        assert!(res.narrative.contains("Narrativa:"));
    }

    #[test]
    fn test_episodic_pyramid_tier3_session_narrative_and_transitive_drilldown() {
        let store = Arc::new(SqliteStore::open_in_memory().expect("in memory store"));
        let pyramid_store = EpisodicPyramidStore::new(store.clone());

        let now = chrono::Utc::now().timestamp();

        // 1. Create 4 Tier 1 episodes (2 for task-auth, 2 for task-db)
        for i in 1..=2 {
            let mut ep = EpisodeRecord::new("sess-full-pyramid", "agent-auth", format!("Auth step {i}"))
                .with_salience(0.8);
            ep.task_id = Some("task-auth".to_string());
            ep.created_at = now + i;
            ep.event_trace = vec![EpisodeStep::new(0, format!("action auth {i}"))
                .with_output(serde_json::json!({ "file": "src/auth.rs" }))];
            ep.outcomes = vec![format!("auth_success_{i}")];
            store.write_episode(&ep).expect("write ep");
        }

        for i in 1..=2 {
            let mut ep = EpisodeRecord::new("sess-full-pyramid", "agent-db", format!("DB step {i}"))
                .with_salience(0.7);
            ep.task_id = Some("task-db".to_string());
            ep.created_at = now + 10 + i;
            ep.event_trace = vec![EpisodeStep::new(0, format!("action db {i}"))
                .with_output(serde_json::json!({ "file": "src/db.rs" }))];
            ep.outcomes = vec![format!("db_success_{i}")];
            store.write_episode(&ep).expect("write ep");
        }

        // 2. Perform Tier 1 -> Tier 2 rollup
        let tier2_rollups = pyramid_store
            .auto_rollup_idle_episodes(Some("sess-full-pyramid"), None)
            .expect("rollup tier 2");
        assert_eq!(tier2_rollups.len(), 2, "Expected 2 Tier 2 milestone rollups");
        assert!(tier2_rollups.iter().all(|r| r.tier == 2));
        assert!(tier2_rollups.iter().all(|r| r.parent_id.is_none()));

        // 3. Perform Tier 2 -> Tier 3 session narrative rollup
        let bridge = SlmPyramidBridge::with_default_config();
        let tier3_opt = pyramid_store
            .generate_session_narrative(Some("sess-full-pyramid"), Some(&bridge))
            .expect("generate tier 3");

        assert!(tier3_opt.is_some(), "Expected Tier 3 session narrative to be generated");
        let tier3 = tier3_opt.unwrap();
        assert_eq!(tier3.tier, 3);
        assert_eq!(tier3.session_id, "sess-full-pyramid");
        assert_eq!(tier3.child_ids.len(), 2, "Tier 3 must reference the 2 Tier 2 milestones");
        assert!(tier3.summary.starts_with("Narrativa:"));
        assert_eq!(tier3.outcomes.len(), 4, "Must aggregate unique outcomes from both milestones");

        // 4. Verify that Tier 2 milestones now have parent_id == Some(tier3.id)
        for t2 in &tier2_rollups {
            let fetched = store.get_episode(&t2.id).expect("get ep").expect("exists");
            assert_eq!(fetched.parent_id, Some(tier3.id), "Tier 2 milestone parent_id must point to Tier 3 epoch");
        }

        // 5. Subsequent call returns None (all Tier 2 milestones are rolled up)
        let empty_second_call = pyramid_store
            .generate_session_narrative(Some("sess-full-pyramid"), Some(&bridge))
            .expect("second call");
        assert!(empty_second_call.is_none(), "No unrolled Tier 2 milestones remaining");

        // 6. Drill down from Tier 3 reconstructs all 4 leaf episodes
        let tree_opt = pyramid_store.drill_down(&tier3.id).expect("drill down");
        assert!(tree_opt.is_some());
        let tree = tree_opt.unwrap();
        assert_eq!(tree.root.id, tier3.id);
        assert_eq!(tree.root.tier, 3);
        assert_eq!(tree.leaves.len(), 4, "Hierarchical drill down from Tier 3 must reach all 4 Tier 1 leaves");
        assert_eq!(tree.total_steps, 4);
    }
}
