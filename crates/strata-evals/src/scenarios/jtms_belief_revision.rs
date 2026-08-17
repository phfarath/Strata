use anyhow::{bail, Result};
use strata_core::{
    schemas::{FactStatus, SemanticFact},
    state::Scope,
};
use strata_memory::{EmbeddingProvider, MockEmbeddingProvider, SqliteStore, TruthMaintenanceSystem};

/// Scenario 4: JTMS Contradiction Detection & Non-Monotonic Belief Revision
/// Evaluates:
/// 1. Ingestion of baseline Fact 1 (REST JSON API architecture).
/// 2. Ingestion of contradictory Fact 2 (gRPC Protobuf architecture migration).
/// 3. Justification-Based Truth Maintenance System (JTMS) detects topic collision and supersedes Fact 1.
/// 4. Fact 1 is marked Deprecated with pointer to Fact 2 (`replaced_by`).
/// 5. Fact 2 is Active with incremented version (`version: 2`).
pub async fn run_jtms_belief_revision_scenario() -> Result<()> {
    println!("\n▶ Running Eval Scenario: JTMS Contradiction Detection & Belief Revision");

    // 1. Setup in-memory store and JTMS engine
    let store = SqliteStore::open_in_memory()?;
    let jtms = TruthMaintenanceSystem::with_default_threshold();
    let embedder = MockEmbeddingProvider::default();

    // 2. Ingest Fact 1 (Baseline Truth)
    let fact1_text = "The backend microservices communication layer is implemented using REST JSON APIs with OpenAPI 3.0 specifications.";
    let mut fact1 = SemanticFact::new(
        fact1_text,
        "backend_architecture",
        Scope::Project("strata-core".to_string()),
    )
    .with_importance(0.85)
    .with_confidence(0.95);

    let emb1 = embedder.embed_text(fact1_text).await?;
    let initial_conflicts = jtms.resolve_and_upsert(&store, &mut fact1, &emb1)?;

    println!("  [Ingestion 1: Baseline Fact]");
    println!("    • Fact ID:    {}", fact1.id);
    println!("    • Statement:  '{}'", fact1.statement);
    println!("    • Status:     {:?}", fact1.status);
    println!("    • Version:    {}", fact1.version);
    println!("    • Conflicts:  {}", initial_conflicts.len());

    if !initial_conflicts.is_empty() {
        bail!("First fact ingestion should have zero conflicts");
    }
    if fact1.status != FactStatus::Active || fact1.version != 1 {
        bail!("First fact should be Active with version 1");
    }

    // 3. Ingest Fact 2 (Contradictory / Superseding Truth)
    // Same category and project scope, describing a migration/replacement
    let fact2_text = "The backend microservices communication layer is migrated to gRPC Protobuf, deprecating REST JSON APIs.";
    let mut fact2 = SemanticFact::new(
        fact2_text,
        "backend_architecture",
        Scope::Project("strata-core".to_string()),
    )
    .with_importance(0.90)
    .with_confidence(0.98);

    let emb2 = embedder.embed_text(fact2_text).await?;
    let revision_conflicts = jtms.resolve_and_upsert(&store, &mut fact2, &emb2)?;

    println!("\n  [Ingestion 2: Superseding Migration Fact]");
    println!("    • Fact ID:    {}", fact2.id);
    println!("    • Statement:  '{}'", fact2.statement);
    println!("    • Status:     {:?}", fact2.status);
    println!("    • Version:    {}", fact2.version);
    println!("    • Conflicts Detected: {}", revision_conflicts.len());

    if revision_conflicts.is_empty() {
        bail!("JTMS should have detected conflict between REST API baseline and gRPC migration");
    }

    // 4. Verify Fact 1 in SQLite is now Deprecated and points to Fact 2
    let retrieved_fact1 = store.get_semantic_fact(&fact1.id)?.expect("Fact 1 must exist in store");
    println!("\n  [Post-Revision State Check]");
    println!("    • Fact 1 Status:      {:?}", retrieved_fact1.status);
    println!("    • Fact 1 Replaced By: {:?}", retrieved_fact1.replaced_by);
    println!("    • Fact 2 Status:      {:?}", fact2.status);
    println!("    • Fact 2 Version:     {}", fact2.version);

    if retrieved_fact1.status != FactStatus::Deprecated {
        bail!("Fact 1 status was not changed to Deprecated");
    }

    if retrieved_fact1.replaced_by != Some(fact2.id) {
        bail!(
            "Fact 1 replaced_by should point to Fact 2 ({}), found: {:?}",
            fact2.id,
            retrieved_fact1.replaced_by
        );
    }

    if fact2.status != FactStatus::Active {
        bail!("Fact 2 should be Active");
    }

    if fact2.version < 2 {
        bail!("Fact 2 version should be incremented to 2, found: {}", fact2.version);
    }

    // 5. Query active facts: Only Fact 2 should be returned
    let active_facts = store.get_all_semantic_facts(None, Some(FactStatus::Active), 10)?;
    if active_facts.len() != 1 || active_facts[0].id != fact2.id {
        bail!("Active facts query should only return Fact 2");
    }

    println!("  ✓ JTMS belief revision and non-monotonic update verified successfully!");
    Ok(())
}
