use anyhow::{bail, Result};
use chrono::{Duration, Utc};
use strata_core::{
    schemas::{FactStatus, SemanticFact},
    state::{MemoryRecord, MemoryType, Scope},
};
use strata_memory::{DecayCalculator, SqliteStore};

/// Scenario 3: Mathematical ACT-R Memory Decay & Ebbinghaus Curve Simulation
/// Evaluates:
/// 1. High-importance memories (0.95) retain activation across 30 days.
/// 2. Access frequency boosts stability and retention according to ACT-R power-law.
/// 3. Low-importance (0.15), unaccessed memories decay below threshold and are pruned.
pub async fn run_decay_curve_simulation_scenario() -> Result<()> {
    println!("\n▶ Running Eval Scenario: Mathematical ACT-R Memory Decay & Curve Simulation");

    let calculator = DecayCalculator::with_default_config();

    // 1. ACT-R Activation across time intervals: 1 hr, 1 day, 7 days, 30 days
    let hours_1 = 1.0f32;
    let hours_24 = 24.0f32;
    let hours_168 = 168.0f32;
    let hours_720 = 720.0f32; // 30 days

    let act_1hr = calculator.calculate_act_r_activation(&[hours_1], 0.95, 1.0);
    let act_1day = calculator.calculate_act_r_activation(&[hours_24], 0.95, 1.0);
    let act_7days = calculator.calculate_act_r_activation(&[hours_168], 0.95, 1.0);
    let act_30days = calculator.calculate_act_r_activation(&[hours_720], 0.95, 1.0);

    println!("  [ACT-R Activation (Importance=0.95)]");
    println!("    • 1 hour:   {act_1hr:.3}");
    println!("    • 1 day:    {act_1day:.3}");
    println!("    • 7 days:   {act_7days:.3}");
    println!("    • 30 days:  {act_30days:.3}");

    if act_1hr <= act_1day || act_1day <= act_7days || act_7days <= act_30days {
        bail!("ACT-R activation must monotonically decay over time: 1hr={act_1hr}, 1day={act_1day}, 7d={act_7days}, 30d={act_30days}");
    }

    let low_act_30days = calculator.calculate_act_r_activation(&[hours_720], 0.15, 0.5);
    if act_30days <= low_act_30days {
        bail!("High importance memory must maintain higher activation than low importance memory: {act_30days} vs {low_act_30days}");
    }

    // 2. Frequency access boost validation
    let stab_unaccessed = calculator.calculate_stability(0, 0.5);
    let stab_accessed_5 = calculator.calculate_stability(5, 0.5);
    let stab_accessed_20 = calculator.calculate_stability(20, 0.5);

    println!("  [Stability vs Access Count (Importance=0.5)]");
    println!("    • 0 accesses:  {stab_unaccessed:.2} hrs");
    println!("    • 5 accesses:  {stab_accessed_5:.2} hrs");
    println!("    • 20 accesses: {stab_accessed_20:.2} hrs");

    if stab_accessed_5 <= stab_unaccessed {
        bail!("5 accesses must increase stability over 0 accesses");
    }
    if stab_accessed_20 <= stab_accessed_5 {
        bail!("20 accesses must increase stability over 5 accesses");
    }

    // 3. Ebbinghaus retention comparison at 7 days (168 hours)
    let ret_unaccessed_7d = calculator.calculate_ebbinghaus_retention(hours_168, stab_unaccessed);
    let ret_accessed_7d = calculator.calculate_ebbinghaus_retention(hours_168, stab_accessed_20);

    println!("  [7-Day Ebbinghaus Retention]");
    println!("    • Unaccessed: {ret_unaccessed_7d:.4}");
    println!("    • 20 Accesses: {ret_accessed_7d:.4}");

    if ret_accessed_7d <= ret_unaccessed_7d {
        bail!("Accessed memory must have higher 7-day retention than unaccessed memory");
    }

    // 4. In-Memory SQLite Pruning Simulation
    let store = SqliteStore::open_in_memory()?;
    let now = Utc::now();
    let month_ago = now - Duration::days(30);

    // Insert high-importance invariant fact (0.95)
    let mut high_fact = SemanticFact::new(
        "Core SQLite WAL database connection settings",
        "configuration",
        Scope::Global,
    )
    .with_importance(0.95)
    .with_confidence(1.0);
    high_fact.created_at = month_ago;
    high_fact.last_updated_at = month_ago;
    store.insert_or_update_semantic_fact(&high_fact)?;

    // Insert low-importance ephemeral fact (0.15) from 30 days ago
    let mut low_fact = SemanticFact::new(
        "Temporary scratchpad note for ticket #1024",
        "note",
        Scope::Global,
    )
    .with_importance(0.15)
    .with_confidence(0.5);
    low_fact.created_at = month_ago;
    low_fact.last_updated_at = month_ago;
    store.insert_or_update_semantic_fact(&low_fact)?;

    // Insert low-importance memory record (0.15) from 30 days ago
    let mut low_record = MemoryRecord::new(
        MemoryType::Episodic,
        "Temporary build artifact log",
        Scope::Global,
    )
    .with_importance(0.15)
    .with_confidence(0.5);
    low_record.created_at = month_ago;
    let _ = store.insert_or_update_memory(&low_record)?;

    // Run pruning with threshold = 0.25
    let prune_report = calculator.prune_expired(&store, Some(0.25), Some(now))?;
    println!("  [Prune Execution Report]");
    println!("    • Facts Pruned:    {}", prune_report.facts_pruned);
    println!("    • Memories Pruned: {}", prune_report.memories_pruned);

    if prune_report.facts_pruned == 0 && prune_report.memories_pruned == 0 {
        bail!("Expected 30-day-old low importance memories to be pruned");
    }

    // Verify high-importance fact is still Active
    let active_facts = store.get_all_semantic_facts(None, Some(FactStatus::Active), 100)?;
    if !active_facts.iter().any(|f| f.id == high_fact.id) {
        bail!("High importance fact (0.95) was unexpectedly pruned");
    }

    // Verify low-importance fact was deprecated
    let deprecated_facts = store.get_all_semantic_facts(None, Some(FactStatus::Deprecated), 100)?;
    if !deprecated_facts.iter().any(|f| f.id == low_fact.id) {
        bail!("Low importance fact (0.15) was not marked Deprecated");
    }

    println!("  ✓ Decay curve simulation and pruning verified successfully!");
    Ok(())
}
