use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use strata_core::events::Event;

use crate::embedding::cosine_similarity;

/// Decision made by the Subconscious Gating mechanism.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum GatingDecision {
    /// Explicit high-importance event (importance >= bypass_threshold); bypasses recurrence and consolidates immediately.
    BypassImmediate { reason: String },
    /// Cluster has met sustained recurrence threshold and is ready for consolidation.
    Consolidate {
        cluster_id: String,
        recurrence_count: usize,
        recurrence_density: f32,
    },
    /// Event held in subconscious buffer pending future recurrence.
    HoldInBuffer {
        cluster_id: String,
        current_count: usize,
        needed: usize,
    },
}

/// Configuration for the subconscious recurrence buffer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SubconsciousConfig {
    /// Importance threshold above which events bypass recurrence checks (default 0.9).
    pub bypass_importance_threshold: f32,
    /// Minimum recurrence count required to trigger consolidation (default 2).
    pub min_recurrence_count: usize,
    /// Cosine similarity threshold to consider events part of the same recurrence cluster (default 0.80).
    pub cluster_similarity_threshold: f32,
    /// Half-life in hours for temporal decay when calculating recurrence density (default 24.0).
    pub temporal_sigma_hours: f32,
    /// Minimum recurrence density score required to consolidate (default 1.5).
    pub min_density_score: f32,
}

impl Default for SubconsciousConfig {
    fn default() -> Self {
        Self {
            bypass_importance_threshold: 0.9,
            min_recurrence_count: 2,
            cluster_similarity_threshold: 0.80,
            temporal_sigma_hours: 24.0,
            min_density_score: 1.5,
        }
    }
}

/// An entry in the subconscious buffer with precomputed embedding.
#[derive(Debug, Clone)]
pub struct SubconsciousEntry {
    pub event: Event,
    pub embedding: Vec<f32>,
    pub importance: f32,
    pub timestamp: DateTime<Utc>,
}

/// A cluster of recurring events in the subconscious buffer.
#[derive(Debug, Clone, Default)]
pub struct RecurrenceCluster {
    pub id: String,
    pub centroid: Vec<f32>,
    pub entries: Vec<SubconsciousEntry>,
}

impl RecurrenceCluster {
    pub fn new(id: String, first_entry: SubconsciousEntry) -> Self {
        let centroid = first_entry.embedding.clone();
        Self {
            id,
            centroid,
            entries: vec![first_entry],
        }
    }

    /// Add an entry and update centroid online.
    pub fn add_entry(&mut self, entry: SubconsciousEntry) {
        let n = self.entries.len() as f32;
        for (i, val) in entry.embedding.iter().enumerate() {
            if i < self.centroid.len() {
                self.centroid[i] = (self.centroid[i] * n + val) / (n + 1.0);
            }
        }
        self.entries.push(entry);
    }

    /// Calculate recurrence density R(M) = sum_{e in M} exp(-dt / sigma) * cosine(e, centroid)
    pub fn calculate_density(&self, now: DateTime<Utc>, sigma_hours: f32) -> f32 {
        let mut density = 0.0f32;
        let sigma_seconds = (sigma_hours * 3600.0).max(1.0);

        for entry in &self.entries {
            let dt_seconds = (now - entry.timestamp).num_seconds().max(0) as f32;
            let temporal_factor = (-dt_seconds / sigma_seconds).exp();
            let sim = cosine_similarity(&entry.embedding, &self.centroid).max(0.0);
            density += temporal_factor * sim;
        }

        density
    }
}

/// Subconscious event buffer inspired by RecMem (arXiv:2605.16045).
/// Holds fast, low-cost events and triggers consolidation only upon sustained recurrence or explicit high importance.
pub struct SubconsciousBuffer {
    pub config: SubconsciousConfig,
    pub clusters: HashMap<String, RecurrenceCluster>,
    pub bypass_queue: Vec<SubconsciousEntry>,
    next_cluster_id: usize,
}

impl SubconsciousBuffer {
    pub fn new(config: SubconsciousConfig) -> Self {
        Self {
            config,
            clusters: HashMap::new(),
            bypass_queue: Vec::new(),
            next_cluster_id: 1,
        }
    }

    pub fn with_default_config() -> Self {
        Self::new(SubconsciousConfig::default())
    }

    /// Ingest an event and evaluate gating decision.
    pub fn ingest(&mut self, event: Event, embedding: Vec<f32>, importance: f32) -> GatingDecision {
        let now = Utc::now();
        let entry = SubconsciousEntry {
            event: event.clone(),
            embedding: embedding.clone(),
            importance,
            timestamp: event.timestamp,
        };

        // 1. Bypass rule: Explicit high-importance events consolidate immediately
        if importance >= self.config.bypass_importance_threshold {
            self.bypass_queue.push(entry);
            return GatingDecision::BypassImmediate {
                reason: format!(
                    "Event importance {:.2} >= bypass threshold {:.2}",
                    importance, self.config.bypass_importance_threshold
                ),
            };
        }

        // 2. Find best matching cluster in subconscious buffer
        let mut best_cluster_id = None;
        let mut best_sim = 0.0f32;

        for (id, cluster) in &self.clusters {
            let sim = cosine_similarity(&embedding, &cluster.centroid);
            if sim > best_sim && sim >= self.config.cluster_similarity_threshold {
                best_sim = sim;
                best_cluster_id = Some(id.clone());
            }
        }

        let cluster_id = match best_cluster_id {
            Some(cid) => {
                if let Some(c) = self.clusters.get_mut(&cid) {
                    c.add_entry(entry);
                }
                cid
            }
            None => {
                let cid = format!("cluster-{}", self.next_cluster_id);
                self.next_cluster_id += 1;
                self.clusters
                    .insert(cid.clone(), RecurrenceCluster::new(cid.clone(), entry));
                cid
            }
        };

        // 3. Evaluate recurrence density
        let cluster = self.clusters.get(&cluster_id).unwrap();
        let count = cluster.entries.len();
        let density = cluster.calculate_density(now, self.config.temporal_sigma_hours);

        if count >= self.config.min_recurrence_count && density >= self.config.min_density_score {
            GatingDecision::Consolidate {
                cluster_id,
                recurrence_count: count,
                recurrence_density: density,
            }
        } else {
            GatingDecision::HoldInBuffer {
                cluster_id,
                current_count: count,
                needed: self.config.min_recurrence_count,
            }
        }
    }

    /// Extract events ready for consolidation from a specific cluster.
    pub fn drain_cluster(&mut self, cluster_id: &str) -> Option<Vec<SubconsciousEntry>> {
        self.clusters.remove(cluster_id).map(|c| c.entries)
    }

    /// Extract all bypass items.
    pub fn drain_bypass(&mut self) -> Vec<SubconsciousEntry> {
        std::mem::take(&mut self.bypass_queue)
    }

    /// Drain all clusters that have reached the recurrence threshold.
    pub fn drain_ready_clusters(&mut self) -> Vec<(String, Vec<SubconsciousEntry>)> {
        let now = Utc::now();
        let mut ready_ids = Vec::new();

        for (id, cluster) in &self.clusters {
            let count = cluster.entries.len();
            let density = cluster.calculate_density(now, self.config.temporal_sigma_hours);
            if count >= self.config.min_recurrence_count && density >= self.config.min_density_score
            {
                ready_ids.push(id.clone());
            }
        }

        let mut result = Vec::new();
        for id in ready_ids {
            if let Some(entries) = self.drain_cluster(&id) {
                result.push((id, entries));
            }
        }
        result
    }
}
