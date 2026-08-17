use chrono::Utc;
use serde::{Deserialize, Serialize};
use strata_core::errors::StrataError;
use strata_core::schemas::{FactStatus, SemanticFact};
use uuid::Uuid;

use crate::embedding::cosine_similarity;
use crate::store::SqliteStore;

/// Resolution strategy when a conflict is detected between semantic facts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ConflictResolution {
    /// Supersede old fact: mark old fact Deprecated (OUT) pointing to new fact, make new fact Active (IN) with incremented version.
    Supersede,
    /// Reject new candidate: old fact remains Active (IN), candidate is marked Deprecated / Outlier.
    Reject,
    /// Coexist: both facts remain Active (IN) under distinct conditions or scopes.
    Coexist,
}

/// Represents a detected conflict match between an existing fact and a new candidate fact.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConflictMatch {
    pub existing_fact_id: Uuid,
    pub existing_statement: String,
    pub similarity: f32,
    pub has_lexical_contradiction: bool,
    pub contradiction_cues: Vec<String>,
}

/// Justification-based Truth Maintenance System (JTMS) & Belief Revision engine.
#[derive(Debug, Clone)]
pub struct TruthMaintenanceSystem {
    /// Cosine similarity threshold above which statements are considered semantic candidates for conflict
    pub similarity_threshold: f32,
}

impl TruthMaintenanceSystem {
    pub fn new(similarity_threshold: f32) -> Self {
        Self {
            similarity_threshold: similarity_threshold.clamp(0.0, 1.0),
        }
    }

    pub fn with_default_threshold() -> Self {
        Self::new(0.85)
    }

    /// Detect lexical contradiction cues between two statements.
    pub fn detect_lexical_contradiction(&self, text_a: &str, text_b: &str) -> (bool, Vec<String>) {
        let a_lower = text_a.to_lowercase();
        let b_lower = text_b.to_lowercase();

        let mut cues = Vec::new();

        // Antonym, protocol, and polarity pairs
        let antonym_pairs = [
            ("always", "never"),
            ("enable", "disable"),
            ("enabled", "disabled"),
            ("true", "false"),
            ("use", "avoid"),
            ("use", "do not use"),
            ("supported", "unsupported"),
            ("deprecated", "active"),
            ("required", "optional"),
            ("allow", "deny"),
            ("allowed", "forbidden"),
            ("increase", "decrease"),
            ("must", "must not"),
            ("should", "should not"),
            ("can", "cannot"),
            ("is", "is not"),
            ("valid", "invalid"),
            ("success", "failure"),
            ("rest", "grpc"),
            ("json", "protobuf"),
            ("mysql", "postgres"),
            ("sqlite", "postgres"),
        ];

        for (word1, word2) in antonym_pairs {
            let (has_1_in_a, has_2_in_a) = (a_lower.contains(word1), a_lower.contains(word2));
            let (has_1_in_b, has_2_in_b) = (b_lower.contains(word1), b_lower.contains(word2));

            if (has_1_in_a && has_2_in_b) || (has_2_in_a && has_1_in_b) {
                cues.push(format!("Polarity opposition: '{word1}' vs '{word2}'"));
            }
        }

        // Negation & migration keywords present in only one of the texts
        let negation_words = [
            "not", "never", "no longer", "deprecated", "deprecating", "removed",
            "disabled", "avoid", "cannot", "migrated", "migration", "replaced",
            "replaces", "supersedes", "superseded"
        ];
        for neg in negation_words {
            let in_a = a_lower.contains(neg);
            let in_b = b_lower.contains(neg);
            if in_a != in_b {
                cues.push(format!("Asymmetric negation/migration keyword: '{neg}'"));
            }
        }

        let is_contradiction = !cues.is_empty();
        (is_contradiction, cues)
    }

    /// Find candidate conflicts against a list of existing facts and their embeddings.
    pub fn find_conflicts(
        &self,
        new_fact: &SemanticFact,
        new_embedding: &[f32],
        existing_facts: &[(SemanticFact, Vec<f32>)],
    ) -> Vec<ConflictMatch> {
        let mut conflicts = Vec::new();

        for (existing, emb) in existing_facts {
            // Only check against active facts and distinct IDs
            if existing.id == new_fact.id || existing.status != FactStatus::Active {
                continue;
            }

            // Scope compatibility check (only check if scopes overlap)
            if !existing.scope.is_compatible(&new_fact.scope) {
                continue;
            }

            let sim = cosine_similarity(new_embedding, emb);
            let (lexical_contradiction, cues) =
                self.detect_lexical_contradiction(&existing.statement, &new_fact.statement);

            // Trigger conflict if:
            // 1. High embedding similarity (> threshold) and lexical contradiction cues found
            // 2. Extremely high embedding similarity (> 0.92) with different statements
            // 3. Same category and direct negation
            let is_conflict = (sim >= self.similarity_threshold && (lexical_contradiction || sim >= 0.92))
                || (lexical_contradiction && existing.category == new_fact.category && sim >= 0.40)
                || (lexical_contradiction && cues.len() >= 2);

            if is_conflict {
                conflicts.push(ConflictMatch {
                    existing_fact_id: existing.id,
                    existing_statement: existing.statement.clone(),
                    similarity: sim,
                    has_lexical_contradiction: lexical_contradiction,
                    contradiction_cues: cues,
                });
            }
        }

        conflicts
    }

    /// Apply belief update to SQLite store when revising facts.
    pub fn apply_belief_update(
        &self,
        store: &SqliteStore,
        old_fact_id: &Uuid,
        new_fact: &mut SemanticFact,
        resolution_type: ConflictResolution,
    ) -> Result<(), StrataError> {
        match resolution_type {
            ConflictResolution::Supersede => {
                if let Some(mut old_fact) = store.get_semantic_fact(old_fact_id)? {
                    // Deprecate old fact (OUT)
                    old_fact.status = FactStatus::Deprecated;
                    old_fact.replaced_by = Some(new_fact.id);
                    old_fact.last_updated_at = Utc::now();
                    store.insert_or_update_semantic_fact(&old_fact)?;

                    // Activate new fact (IN) with incremented version
                    new_fact.status = FactStatus::Active;
                    new_fact.version = old_fact.version + 1;
                    new_fact.last_updated_at = Utc::now();
                    store.insert_or_update_semantic_fact(new_fact)?;
                } else {
                    // Old fact not found; simply insert new fact
                    new_fact.status = FactStatus::Active;
                    store.insert_or_update_semantic_fact(new_fact)?;
                }
            }
            ConflictResolution::Reject => {
                // Reject new candidate (OUT / Deprecated)
                new_fact.status = FactStatus::Deprecated;
                new_fact.replaced_by = Some(*old_fact_id);
                new_fact.last_updated_at = Utc::now();
                store.insert_or_update_semantic_fact(new_fact)?;
            }
            ConflictResolution::Coexist => {
                // Keep both active
                new_fact.status = FactStatus::Active;
                store.insert_or_update_semantic_fact(new_fact)?;
            }
        }

        Ok(())
    }

    /// Check for conflicts and automatically resolve & upsert into SQLite store.
    pub fn resolve_and_upsert(
        &self,
        store: &SqliteStore,
        new_fact: &mut SemanticFact,
        new_embedding: &[f32],
    ) -> Result<Vec<ConflictMatch>, StrataError> {
        // Fetch existing active facts with embeddings
        let existing = store.get_semantic_facts_with_embeddings(None, Some(FactStatus::Active))?;
        let active_with_embs: Vec<(SemanticFact, Vec<f32>)> = existing
            .into_iter()
            .filter_map(|(f, e)| e.map(|vec| (f, vec)))
            .collect();

        let conflicts = self.find_conflicts(new_fact, new_embedding, &active_with_embs);

        if let Some(first_conflict) = conflicts.first() {
            // Default automated resolution: higher confidence/newer evidence supersedes older fact
            self.apply_belief_update(
                store,
                &first_conflict.existing_fact_id,
                new_fact,
                ConflictResolution::Supersede,
            )?;
        } else {
            // No conflict: standard active insertion
            new_fact.status = FactStatus::Active;
            store.insert_or_update_semantic_fact(new_fact)?;
        }

        // Store embedding
        store.update_semantic_fact_embedding(&new_fact.id, new_embedding)?;

        Ok(conflicts)
    }
}
