use serde::{Deserialize, Serialize};
use std::collections::HashSet;

use super::pipeline::TrainingPipeline;

/// Functional stratum of an experience replay sample for continual learning.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ReplayStratum {
    /// Foundational / previously mastered domain tasks to prevent catastrophic forgetting.
    HistoricalBaseline,
    /// Newly acquired domain trajectories for forward transfer.
    NovelDomain,
    /// High prediction-error / verified failure mitigations and contrastive hard negatives.
    HardNegative,
}

/// A curated experience replay sample stored in the continual learning buffer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReplaySample {
    pub id: String,
    pub domain: String,
    pub stratum: ReplayStratum,
    pub jsonl_payload: String,
    pub salience: f32,
    pub prediction_error: f32,
    pub historical_pass_rate: f32,
}

impl ReplaySample {
    pub fn new(
        id: impl Into<String>,
        domain: impl Into<String>,
        stratum: ReplayStratum,
        jsonl_payload: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            domain: domain.into(),
            stratum,
            jsonl_payload: jsonl_payload.into(),
            salience: 0.5,
            prediction_error: 0.0,
            historical_pass_rate: 1.0,
        }
    }

    pub fn with_salience(mut self, salience: f32) -> Self {
        self.salience = salience.clamp(0.0, 1.0);
        self
    }

    pub fn with_prediction_error(mut self, error: f32) -> Self {
        self.prediction_error = error.clamp(0.0, 1.0);
        self
    }

    pub fn with_historical_pass_rate(mut self, rate: f32) -> Self {
        self.historical_pass_rate = rate.clamp(0.0, 1.0);
        self
    }

    /// Composite priority score: 0.5 * salience + 0.5 * prediction_error
    pub fn priority(&self) -> f32 {
        0.5 * self.salience + 0.5 * self.prediction_error
    }
}

/// Configuration parameters for the experience replay buffer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReplayBufferConfig {
    pub max_capacity: usize,
    pub historical_ratio: f32,
    pub novel_ratio: f32,
    pub hard_negative_ratio: f32,
    pub priority_alpha: f32,
    pub min_backward_transfer_retention: f32,
    pub seed: u64,
}

impl Default for ReplayBufferConfig {
    fn default() -> Self {
        Self {
            max_capacity: 1000,
            historical_ratio: 0.40,
            novel_ratio: 0.40,
            hard_negative_ratio: 0.20,
            priority_alpha: 0.6,
            min_backward_transfer_retention: 0.95,
            seed: 42,
        }
    }
}

/// Metrics measuring backward retention and forward transfer across adaptation rounds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContinualTransferMetrics {
    pub backward_transfer_retention: f32,
    pub forward_transfer_gain: f32,
    pub forgetting_rate: f32,
    pub historical_samples_count: usize,
    pub novel_samples_count: usize,
    pub hard_negative_samples_count: usize,
    pub catastrophic_forgetting_prevented: bool,
}

/// Curated experience replay buffer with stratified partitioning and priority eviction.
#[derive(Debug, Clone)]
pub struct ExperienceReplayBuffer {
    config: ReplayBufferConfig,
    historical_samples: Vec<ReplaySample>,
    novel_samples: Vec<ReplaySample>,
    hard_negative_samples: Vec<ReplaySample>,
    seen_keys: HashSet<String>,
    seen_ids: HashSet<String>,
}

impl ExperienceReplayBuffer {
    pub fn new(config: ReplayBufferConfig) -> Self {
        Self {
            config,
            historical_samples: Vec::new(),
            novel_samples: Vec::new(),
            hard_negative_samples: Vec::new(),
            seen_keys: HashSet::new(),
            seen_ids: HashSet::new(),
        }
    }

    pub fn with_default_config() -> Self {
        Self::new(ReplayBufferConfig::default())
    }

    pub fn config(&self) -> &ReplayBufferConfig {
        &self.config
    }

    pub fn len(&self) -> usize {
        self.historical_samples.len() + self.novel_samples.len() + self.hard_negative_samples.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn clear(&mut self) {
        self.historical_samples.clear();
        self.novel_samples.clear();
        self.hard_negative_samples.clear();
        self.seen_keys.clear();
        self.seen_ids.clear();
    }

    pub fn get_stratum_count(&self, stratum: ReplayStratum) -> usize {
        match stratum {
            ReplayStratum::HistoricalBaseline => self.historical_samples.len(),
            ReplayStratum::NovelDomain => self.novel_samples.len(),
            ReplayStratum::HardNegative => self.hard_negative_samples.len(),
        }
    }

    pub fn stratum_capacity(&self, stratum: ReplayStratum) -> usize {
        if self.config.max_capacity == 0 {
            return 0;
        }
        let ratio = match stratum {
            ReplayStratum::HistoricalBaseline => self.config.historical_ratio,
            ReplayStratum::NovelDomain => self.config.novel_ratio,
            ReplayStratum::HardNegative => self.config.hard_negative_ratio,
        };
        let cap = ((self.config.max_capacity as f32) * ratio).round() as usize;
        cap.max(1)
    }

    /// Insert a new experience sample into the buffer.
    ///
    /// Deduplicates semantically via `TrainingPipeline::extract_semantic_key`.
    /// When stratum capacity is exceeded, evicts the lowest-priority item.
    /// Returns `true` if the sample is accepted and retained in the buffer.
    pub fn insert(&mut self, sample: ReplaySample) -> bool {
        let semantic_key = TrainingPipeline::extract_semantic_key(&sample.jsonl_payload);
        if self.seen_keys.contains(&semantic_key) || self.seen_ids.contains(&sample.id) {
            return false;
        }

        let stratum = sample.stratum;
        let cap = self.stratum_capacity(stratum);

        let target_vec = match stratum {
            ReplayStratum::HistoricalBaseline => &mut self.historical_samples,
            ReplayStratum::NovelDomain => &mut self.novel_samples,
            ReplayStratum::HardNegative => &mut self.hard_negative_samples,
        };

        self.seen_keys.insert(semantic_key);
        self.seen_ids.insert(sample.id.clone());
        target_vec.push(sample);

        // Check if eviction is required
        if target_vec.len() > cap {
            // Find lowest priority sample
            let mut min_idx = 0;
            let mut min_pri = target_vec[0].priority();

            for (i, item) in target_vec.iter().enumerate().skip(1) {
                let pri = item.priority();
                if pri < min_pri {
                    min_pri = pri;
                    min_idx = i;
                }
            }

            let evicted = target_vec.remove(min_idx);
            let evicted_key = TrainingPipeline::extract_semantic_key(&evicted.jsonl_payload);
            self.seen_keys.remove(&evicted_key);
            self.seen_ids.remove(&evicted.id);

            // If the item evicted was the one we just tried to push
            if min_idx == target_vec.len() {
                return false;
            }
        }

        true
    }

    fn deterministic_tie_breaker(id: &str, seed: u64) -> u64 {
        let mut state = seed ^ 0x517cc1b727220a95;
        for b in id.bytes() {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(b as u64);
        }
        state
    }

    /// Sample a stratified rehearsal batch deterministically matching configured ratios.
    ///
    /// Backfills from other strata if one stratum has fewer samples than its quota.
    pub fn sample_stratified_batch(&self, batch_size: usize) -> Vec<ReplaySample> {
        if batch_size == 0 || self.is_empty() {
            return Vec::new();
        }

        let seed = if self.config.seed == 0 {
            42
        } else {
            self.config.seed
        };

        let mut h_sorted = self.historical_samples.clone();
        let mut n_sorted = self.novel_samples.clone();
        let mut hn_sorted = self.hard_negative_samples.clone();

        let sort_fn = |a: &ReplaySample, b: &ReplaySample| {
            b.priority()
                .partial_cmp(&a.priority())
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| {
                    let hash_a = Self::deterministic_tie_breaker(&a.id, seed);
                    let hash_b = Self::deterministic_tie_breaker(&b.id, seed);
                    hash_b.cmp(&hash_a)
                })
                .then_with(|| a.id.cmp(&b.id))
        };

        h_sorted.sort_by(sort_fn);
        n_sorted.sort_by(sort_fn);
        hn_sorted.sort_by(sort_fn);

        // Compute quota targets
        let h_target = ((batch_size as f32) * self.config.historical_ratio).round() as usize;
        let n_target = ((batch_size as f32) * self.config.novel_ratio).round() as usize;
        let hn_target = batch_size.saturating_sub(h_target + n_target);

        let h_take = h_target.min(h_sorted.len());
        let n_take = n_target.min(n_sorted.len());
        let hn_take = hn_target.min(hn_sorted.len());

        let mut result = Vec::with_capacity(batch_size.min(self.len()));
        result.extend(h_sorted.drain(..h_take));
        result.extend(n_sorted.drain(..n_take));
        result.extend(hn_sorted.drain(..hn_take));

        // Backfill if needed to reach batch_size or full capacity
        let needed = (batch_size.min(self.len())).saturating_sub(result.len());
        if needed > 0 {
            let mut remaining = Vec::new();
            remaining.extend(h_sorted);
            remaining.extend(n_sorted);
            remaining.extend(hn_sorted);
            remaining.sort_by(sort_fn);

            let backfill_count = needed.min(remaining.len());
            result.extend(remaining.drain(..backfill_count));
        }

        result
    }

    /// Export the sampled rehearsal batch as newline-delimited JSONL.
    pub fn export_rehearsal_jsonl(&self, batch_size: usize) -> String {
        let batch = self.sample_stratified_batch(batch_size);
        let mut lines = Vec::with_capacity(batch.len());
        for s in batch {
            let trimmed = s.jsonl_payload.trim();
            if !trimmed.is_empty() {
                lines.push(trimmed.to_string());
            }
        }
        if lines.is_empty() {
            String::new()
        } else {
            lines.join("\n") + "\n"
        }
    }

    /// Evaluates the continual learning safety gate against catastrophic forgetting.
    pub fn evaluate_transfer_gate(
        &self,
        baseline_historical_acc: f32,
        post_historical_acc: f32,
        baseline_novel_acc: f32,
        post_novel_acc: f32,
    ) -> ContinualTransferMetrics {
        let backward_transfer_retention = post_historical_acc;
        let forgetting_rate = (baseline_historical_acc - post_historical_acc).max(0.0);
        let forward_transfer_gain = post_novel_acc - baseline_novel_acc;
        let catastrophic_forgetting_prevented =
            backward_transfer_retention >= self.config.min_backward_transfer_retention;

        ContinualTransferMetrics {
            backward_transfer_retention,
            forward_transfer_gain,
            forgetting_rate,
            historical_samples_count: self.historical_samples.len(),
            novel_samples_count: self.novel_samples.len(),
            hard_negative_samples_count: self.hard_negative_samples.len(),
            catastrophic_forgetting_prevented,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_deduplication_via_semantic_key() {
        let mut buffer = ExperienceReplayBuffer::with_default_config();

        let s1 = ReplaySample::new(
            "s1",
            "auth",
            ReplayStratum::HistoricalBaseline,
            r#"{"prompt":"login user","chosen":"token ok","rejected":"error"}"#,
        );
        let s2 = ReplaySample::new(
            "s2",
            "auth",
            ReplayStratum::HistoricalBaseline,
            r#"{"prompt":"login user","chosen":"token ok","rejected":"error"}"#,
        );

        assert!(buffer.insert(s1));
        assert_eq!(buffer.len(), 1);

        // Same semantic payload under different ID should be rejected
        assert!(!buffer.insert(s2));
        assert_eq!(buffer.len(), 1);
    }

    #[test]
    fn test_capacity_priority_eviction() {
        let config = ReplayBufferConfig {
            max_capacity: 5,
            historical_ratio: 0.40,    // cap: 2
            novel_ratio: 0.40,         // cap: 2
            hard_negative_ratio: 0.20, // cap: 1
            ..Default::default()
        };
        let mut buffer = ExperienceReplayBuffer::new(config);

        let s_low = ReplaySample::new(
            "h1",
            "domain",
            ReplayStratum::HistoricalBaseline,
            r#"{"prompt":"q1","completion":"a1","label":true}"#,
        )
        .with_salience(0.1)
        .with_prediction_error(0.1); // pri: 0.10

        let s_med = ReplaySample::new(
            "h2",
            "domain",
            ReplayStratum::HistoricalBaseline,
            r#"{"prompt":"q2","completion":"a2","label":true}"#,
        )
        .with_salience(0.5)
        .with_prediction_error(0.5); // pri: 0.50

        let s_high = ReplaySample::new(
            "h3",
            "domain",
            ReplayStratum::HistoricalBaseline,
            r#"{"prompt":"q3","completion":"a3","label":true}"#,
        )
        .with_salience(0.9)
        .with_prediction_error(0.9); // pri: 0.90

        assert!(buffer.insert(s_low));
        assert!(buffer.insert(s_med));
        assert_eq!(
            buffer.get_stratum_count(ReplayStratum::HistoricalBaseline),
            2
        );

        // Inserting s_high exceeds cap (2), so s_low (pri 0.10) must be evicted
        assert!(buffer.insert(s_high));
        assert_eq!(
            buffer.get_stratum_count(ReplayStratum::HistoricalBaseline),
            2
        );

        // Verify remaining samples are s_med and s_high
        let batch = buffer.sample_stratified_batch(2);
        let ids: Vec<&str> = batch.iter().map(|s| s.id.as_str()).collect();
        assert!(ids.contains(&"h3"));
        assert!(ids.contains(&"h2"));
        assert!(!ids.contains(&"h1"));
    }

    #[test]
    fn test_stratified_sampling_ratios_and_backfill() {
        let mut buffer = ExperienceReplayBuffer::with_default_config();

        for i in 0..10 {
            buffer.insert(
                ReplaySample::new(
                    format!("hist_{i}"),
                    "historical",
                    ReplayStratum::HistoricalBaseline,
                    format!(r#"{{"prompt":"hist {i}","chosen":"c","rejected":"r"}}"#),
                )
                .with_salience(0.8),
            );
            buffer.insert(
                ReplaySample::new(
                    format!("novel_{i}"),
                    "novel",
                    ReplayStratum::NovelDomain,
                    format!(r#"{{"prompt":"novel {i}","chosen":"c","rejected":"r"}}"#),
                )
                .with_salience(0.7),
            );
            buffer.insert(
                ReplaySample::new(
                    format!("hn_{i}"),
                    "failure",
                    ReplayStratum::HardNegative,
                    format!(r#"{{"prompt":"hn {i}","chosen":"c","rejected":"r"}}"#),
                )
                .with_salience(0.9)
                .with_prediction_error(0.9),
            );
        }

        let batch = buffer.sample_stratified_batch(10);
        assert_eq!(batch.len(), 10);

        let h_count = batch
            .iter()
            .filter(|s| s.stratum == ReplayStratum::HistoricalBaseline)
            .count();
        let n_count = batch
            .iter()
            .filter(|s| s.stratum == ReplayStratum::NovelDomain)
            .count();
        let hn_count = batch
            .iter()
            .filter(|s| s.stratum == ReplayStratum::HardNegative)
            .count();

        // 40% historical, 40% novel, 20% hard negative
        assert_eq!(h_count, 4);
        assert_eq!(n_count, 4);
        assert_eq!(hn_count, 2);
    }

    #[test]
    fn test_export_rehearsal_jsonl() {
        let mut buffer = ExperienceReplayBuffer::with_default_config();
        buffer.insert(ReplaySample::new(
            "s1",
            "d1",
            ReplayStratum::HistoricalBaseline,
            r#"{"prompt":"p1","chosen":"c1","rejected":"r1"}"#,
        ));
        buffer.insert(ReplaySample::new(
            "s2",
            "d2",
            ReplayStratum::NovelDomain,
            r#"{"prompt":"p2","chosen":"c2","rejected":"r2"}"#,
        ));

        let jsonl = buffer.export_rehearsal_jsonl(2);
        assert!(jsonl.contains("p1"));
        assert!(jsonl.contains("p2"));
        assert_eq!(jsonl.lines().count(), 2);
    }

    #[test]
    fn test_evaluate_transfer_gate_pass_and_fail() {
        let buffer = ExperienceReplayBuffer::with_default_config();

        // Safe rehearsal run: historical retention = 0.97 >= 0.95
        let metrics_pass = buffer.evaluate_transfer_gate(0.98, 0.97, 0.40, 0.85);
        assert!(metrics_pass.catastrophic_forgetting_prevented);
        assert!((metrics_pass.backward_transfer_retention - 0.97).abs() < 1e-4);
        assert!((metrics_pass.forward_transfer_gain - 0.45).abs() < 1e-4);
        assert!((metrics_pass.forgetting_rate - 0.01).abs() < 1e-4);

        // Naive run: catastrophic forgetting, retention drops to 0.62 < 0.95
        let metrics_fail = buffer.evaluate_transfer_gate(0.98, 0.62, 0.40, 0.88);
        assert!(!metrics_fail.catastrophic_forgetting_prevented);
        assert!((metrics_fail.backward_transfer_retention - 0.62).abs() < 1e-4);
        assert!((metrics_fail.forgetting_rate - 0.36).abs() < 1e-4);
    }
}
