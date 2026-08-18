use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use reqwest::Client;

use serde_json::Value;
use strata_core::errors::StrataError;
use strata_core::events::Event;
use strata_core::schemas::{
    MemoryFeedback, ProceduralSkill, SemanticFact, SyncConfig, SyncDelta, SyncReport,
};
use strata_core::state::{FailurePattern, MemoryRecord};
use uuid::Uuid;

use crate::jtms::{ConflictResolution, TruthMaintenanceSystem};
use crate::store::SqliteStore;

/// Compute deterministic 16-hex version hash for a JSON value.
pub fn compute_version_hash(payload: &Value) -> String {
    let mut hasher = DefaultHasher::new();
    payload.to_string().hash(&mut hasher);
    let hash = hasher.finish();
    format!("{hash:016x}")
}

/// Compute exponential retry backoff duration with base milliseconds and maximum cap.
pub fn calculate_exponential_backoff(retry_count: u32, base_ms: u64, max_ms: u64) -> std::time::Duration {
    let factor = 2_u64.saturating_pow(retry_count.min(10));
    let backoff = base_ms.saturating_mul(factor);
    std::time::Duration::from_millis(backoff.min(max_ms))
}

/// Offline-first CDC Synchronization Engine for Strata memory spaces.

pub struct SyncEngine {
    store: Arc<SqliteStore>,
    config: SyncConfig,
    http_client: Client,
    jtms: TruthMaintenanceSystem,
}

impl SyncEngine {
    /// Create a new `SyncEngine` instance with given SQLite store and configuration.
    pub fn new(store: Arc<SqliteStore>, config: SyncConfig) -> Self {
        let http_client = Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .unwrap_or_default();

        Self {
            store,
            config,
            http_client,
            jtms: TruthMaintenanceSystem::with_default_threshold(),
        }
    }

    /// Access the underlying store.
    pub fn store(&self) -> &SqliteStore {
        &self.store
    }

    /// Access the current sync configuration.
    pub fn config(&self) -> &SyncConfig {
        &self.config
    }

    /// Update the sync configuration.
    pub fn set_config(&mut self, config: SyncConfig) {
        self.config = config;
    }

    /// Push pending deltas from the outbox to remote endpoint or mark synced in offline mode.
    pub async fn push_deltas(&self) -> Result<usize, StrataError> {
        let pending = self
            .store
            .get_pending_deltas(&self.config.workspace_id, self.config.batch_size)?;

        if pending.is_empty() {
            return Ok(0);
        }

        let delta_ids: Vec<Uuid> = pending.iter().map(|d| d.id).collect();

        if let Some(ref endpoint) = self.config.endpoint {
            let mut req = self.http_client.post(endpoint).json(&serde_json::json!({
                "workspace_id": &self.config.workspace_id,
                "deltas": &pending,
            }));

            if let Some(ref token) = self.config.token {
                req = req.bearer_auth(token);
            }

            match req.send().await {
                Ok(resp) => {
                    if resp.status().is_success() {
                        self.store.mark_deltas_synced(&delta_ids)?;
                        Ok(pending.len())
                    } else {
                        let status = resp.status();
                        let body = resp.text().await.unwrap_or_default();
                        let err_msg = format!("HTTP push failed with status {status}: {body}");
                        self.store.record_delta_failure(&delta_ids, &err_msg)?;
                        Err(StrataError::Network(err_msg))
                    }
                }
                Err(e) => {
                    let err_msg = format!("HTTP push network error: {e}");
                    self.store.record_delta_failure(&delta_ids, &err_msg)?;
                    Err(StrataError::Network(err_msg))
                }
            }
        } else {
            // Offline / local sync mode: automatically mark deltas as synced
            self.store.mark_deltas_synced(&delta_ids)?;
            Ok(pending.len())
        }
    }

    /// Pull remote deltas from configured HTTP endpoint.
    pub async fn pull_remote(&self) -> Result<Vec<SyncDelta>, StrataError> {
        let endpoint = match self.config.endpoint.as_ref() {
            Some(ep) => ep,
            None => return Ok(Vec::new()),
        };

        let last_seq = self
            .store
            .get_sync_metadata("last_remote_seq")?
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(0);

        let url = format!("{endpoint}/pull");
        let mut req = self.http_client.get(&url).query(&[
            ("workspace_id", self.config.workspace_id.as_str()),
            ("since_seq", &last_seq.to_string()),
            ("limit", &self.config.batch_size.to_string()),
        ]);

        if let Some(ref token) = self.config.token {
            req = req.bearer_auth(token);
        }

        let resp = req
            .send()
            .await
            .map_err(|e| StrataError::Network(format!("HTTP pull error: {e}")))?;

        if resp.status().is_success() {
            let deltas: Vec<SyncDelta> = resp
                .json()
                .await
                .map_err(|e| StrataError::Serialization(e.to_string()))?;
            Ok(deltas)
        } else {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            Err(StrataError::Network(format!(
                "HTTP pull failed with status {status}: {body}"
            )))
        }
    }

    /// Apply incoming remote deltas to the local SQLite store with JTMS conflict resolution.
    pub async fn pull_deltas(&self, remote_deltas: Vec<SyncDelta>) -> Result<usize, StrataError> {
        let (applied, _conflicts) = self.apply_incoming_deltas(remote_deltas).await?;
        Ok(applied)
    }

    /// Internal implementation to apply deltas and count conflicts.
    async fn apply_incoming_deltas(
        &self,
        remote_deltas: Vec<SyncDelta>,
    ) -> Result<(usize, usize), StrataError> {
        let mut applied_count = 0;
        let mut conflicts_resolved = 0;

        let mut max_seq = self
            .store
            .get_sync_metadata("last_remote_seq")?
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(0);

        for delta in remote_deltas {
            let kind = delta.kind.to_lowercase();
            match kind.as_str() {
                "fact" | "semantic_fact" => {
                    let mut fact: SemanticFact = serde_json::from_value(delta.payload.clone())
                        .map_err(|e| StrataError::Serialization(e.to_string()))?;

                    // Check if local store already has this fact
                    if let Some(existing_fact) = self.store.get_semantic_fact(&fact.id)? {
                        let existing_payload = serde_json::to_value(&existing_fact)?;
                        let existing_hash = compute_version_hash(&existing_payload);

                        // If version hash diverges, invoke JTMS conflict resolution
                        if existing_hash != delta.version_hash {
                            self.jtms.apply_belief_update(
                                &self.store,
                                &existing_fact.id,
                                &mut fact,
                                ConflictResolution::Supersede,
                            )?;
                            conflicts_resolved += 1;
                        } else {
                            self.store.insert_or_update_semantic_fact(&fact)?;
                        }
                    } else {
                        self.store.insert_or_update_semantic_fact(&fact)?;
                    }
                    applied_count += 1;
                }
                "memory" | "memory_record" => {
                    let memory: MemoryRecord = serde_json::from_value(delta.payload.clone())
                        .map_err(|e| StrataError::Serialization(e.to_string()))?;
                    self.store.insert_or_update_memory(&memory)?;
                    applied_count += 1;
                }
                "skill" | "procedural_skill" => {
                    let skill: ProceduralSkill = serde_json::from_value(delta.payload.clone())
                        .map_err(|e| StrataError::Serialization(e.to_string()))?;
                    self.store.insert_or_update_procedural_skill(&skill)?;
                    applied_count += 1;
                }
                "event" => {
                    let event: Event = serde_json::from_value(delta.payload.clone())
                        .map_err(|e| StrataError::Serialization(e.to_string()))?;
                    self.store.insert_event(&event)?;
                    applied_count += 1;
                }
                "feedback" | "memory_feedback" => {
                    let fb: MemoryFeedback = serde_json::from_value(delta.payload.clone())
                        .map_err(|e| StrataError::Serialization(e.to_string()))?;
                    self.store.record_memory_feedback(&fb)?;
                    applied_count += 1;
                }
                "failure" | "failure_pattern" => {
                    let pattern: FailurePattern = serde_json::from_value(delta.payload.clone())
                        .map_err(|e| StrataError::Serialization(e.to_string()))?;
                    self.store.upsert_failure_pattern(&pattern)?;
                    applied_count += 1;
                }
                _ => {
                    // Generic payload: persist delta as synced in outbox
                    let mut synced_delta = delta.clone();
                    synced_delta.synced = true;
                    self.store.enqueue_delta(&synced_delta)?;
                    applied_count += 1;
                }
            }

            if delta.seq > max_seq {
                max_seq = delta.seq;
            }
        }

        if max_seq > 0 {
            self.store
                .set_sync_metadata("last_remote_seq", &max_seq.to_string())?;
        }

        Ok((applied_count, conflicts_resolved))
    }

    /// Execute a complete sync cycle (push pending local deltas, pull remote deltas, compute status).
    pub async fn sync_cycle(&self) -> Result<SyncReport, StrataError> {
        let mut report = SyncReport::default();

        // 1. Push pending deltas
        match self.push_deltas().await {
            Ok(count) => {
                report.pushed_count = count;
            }
            Err(e) => {
                report.errors.push(format!("Push error: {e}"));
            }
        }

        // 2. Pull remote deltas if endpoint configured
        if self.config.endpoint.is_some() {
            match self.pull_remote().await {
                Ok(remote_deltas) => match self.apply_incoming_deltas(remote_deltas).await {
                    Ok((applied, conflicts)) => {
                        report.pulled_count = applied;
                        report.conflicts_resolved = conflicts;
                    }
                    Err(e) => {
                        report.errors.push(format!("Apply remote deltas error: {e}"));
                    }
                },
                Err(e) => {
                    report.errors.push(format!("Pull error: {e}"));
                }
            }
        }

        // 3. Compute last_seq
        let (_pending, max_local_seq) = self.store.get_sync_status(&self.config.workspace_id)?;
        let last_remote_seq = self
            .store
            .get_sync_metadata("last_remote_seq")?
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(0);
        report.last_seq = max_local_seq.max(last_remote_seq);

        Ok(report)
    }
}
