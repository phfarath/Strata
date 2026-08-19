use std::env;
use anyhow::Result;
use clap::Args;
use strata_memory::SqliteStore;
use strata_reasoning::{BlastRadiusReport, CausalNodeKind, PatchSimulationResult, WorldModel};

#[derive(Debug, Args, Clone)]
pub struct BlastRadiusArgs {
    /// File path, struct, module or API endpoint to analyze (e.g. 'crates/strata-server/src/storage.rs')
    #[arg(value_name = "TARGET")]
    pub target: Option<String>,

    /// Maximum traversal depth for transitive dependencies (default: 3)
    #[arg(short = 'd', long, default_value = "3")]
    pub depth: usize,

    /// Multiple targets for patch simulation mode (comma-separated or multiple flags)
    #[arg(long, value_delimiter = ',')]
    pub files: Vec<String>,

    /// Output full analysis in JSON format
    #[arg(long)]
    pub json: bool,
}

pub async fn run_blast_radius(args: BlastRadiusArgs, store: &SqliteStore) -> Result<()> {
    let world_model = WorldModel::new();

    // 1. Link invariants and failure patterns from SQLite
    link_store_invariants(&world_model, store).await?;

    // 2. Index workspace files
    let cwd = env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let _ = world_model.index_workspace(&cwd).await;

    // 3. Multi-file patch simulation vs single target analysis
    if !args.files.is_empty() {
        let sim_result = world_model.simulate_patch(&args.files).await?;
        if args.json {
            println!("{}", serde_json::to_string_pretty(&sim_result)?);
        } else {
            render_patch_simulation(&sim_result);
        }
        return Ok(());
    }

    let target = args.target.unwrap_or_else(|| "crates/strata-server/src/storage.rs".to_string());
    let report = world_model.predict_impact(&target, args.depth).await?;

    if args.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        render_blast_radius_cli(&report);
    }

    Ok(())
}

async fn link_store_invariants(world_model: &WorldModel, store: &SqliteStore) -> Result<()> {
    // High-importance facts
    if let Ok(facts) = store.get_all_semantic_facts(None, Some(strata_core::schemas::FactStatus::Active), 500) {
        for f in facts {
            if f.importance >= 0.90 {
                let _ = world_model.register_invariant(&f.statement, &f.category, "crate:strata_core").await;
            }
        }
    }

    // Failure patterns
    if let Ok(failures) = store.search_failures(None, None, 500) {
        for p in failures {
            let _ = world_model.register_anti_pattern(&p.signature, &p.pattern_name, &p.mitigation, "crate:strata_core").await;
        }
    }

    Ok(())
}

fn render_blast_radius_cli(report: &BlastRadiusReport) {
    println!("\n╔══════════════════════════════════════════════════════════════════════════════╗");
    println!("║           🌐 STRATA CAUSAL WORLD MODEL & BLAST RADIUS INSPECTOR              ║");
    println!("╚══════════════════════════════════════════════════════════════════════════════╝\n");

    println!("🎯 TARGET COMPONENT:  \x1b[1;36m{}\x1b[0m", report.target_name);
    println!("🆔 CANONICAL NODE ID: \x1b[90m{}\x1b[0m", report.target_id);
    println!("🔍 MAX GRAPH DEPTH:   {}", report.max_depth);
    println!("📊 GRAPH COVERAGE:    {} total architectural nodes indexed", report.total_nodes_scanned);

    // Risk score badge
    let risk_pct = (report.overall_risk_score * 100.0).round();
    let risk_badge = if report.overall_risk_score >= 0.80 {
        format!("\x1b[1;31m🚨 CRITICAL RISK ({:.0}%)\x1b[0m", risk_pct)
    } else if report.overall_risk_score >= 0.50 {
        format!("\x1b[1;33m⚠️  ELEVATED RISK ({:.0}%)\x1b[0m", risk_pct)
    } else if report.overall_risk_score >= 0.25 {
        format!("\x1b[1;34m🟡 MODERATE RISK ({:.0}%)\x1b[0m", risk_pct)
    } else {
        format!("\x1b[1;32m🟢 LOW RISK ({:.0}%)\x1b[0m", risk_pct)
    };

    println!("⚡ PRE-CODE BLAST RISK: {}\n", risk_badge);

    println!("────────────────────────────────────────────────────────────────────────────────");
    println!("🌲 CAUSAL DEPENDENCY & IMPACT RIPPLE TREE");
    println!("────────────────────────────────────────────────────────────────────────────────");

    if report.direct_impacts.is_empty() && report.transitive_impacts.is_empty() {
        println!("   └── \x1b[90m(Isolated component — zero upstream callers or consumers detected)\x1b[0m");
    } else {
        for (i, d) in report.direct_impacts.iter().enumerate() {
            let is_last = i == report.direct_impacts.len() - 1 && report.transitive_impacts.is_empty();
            let branch = if is_last { "└──" } else { "├──" };
            let breaking = if d.is_breaking_risk {
                " \x1b[1;31m[BREAKING RISK]\x1b[0m"
            } else {
                ""
            };
            let kind_label = format_node_kind(d.kind);

            println!(
                "   {} \x1b[1m(d=1 DIRECT)\x1b[0m {} {} \x1b[90m(coupling: {:.0}%)\x1b[0m{}",
                branch, kind_label, d.name, d.cumulative_weight * 100.0, breaking
            );
        }

        for (i, t) in report.transitive_impacts.iter().enumerate() {
            let is_last = i == report.transitive_impacts.len() - 1;
            let branch = if is_last { "└──" } else { "├──" };
            let breaking = if t.is_breaking_risk {
                " \x1b[1;31m[BREAKING RISK]\x1b[0m"
            } else {
                ""
            };
            let kind_label = format_node_kind(t.kind);

            println!(
                "   │   {} \x1b[90m(d={} TRANSITIVE)\x1b[0m {} {} \x1b[90m(coupling: {:.0}%)\x1b[0m{}",
                branch, t.distance, kind_label, t.name, t.cumulative_weight * 100.0, breaking
            );
        }
    }

    if !report.triggered_invariants.is_empty() {
        println!("\n────────────────────────────────────────────────────────────────────────────────");
        println!("🛡️  ARCHITECTURAL CONTRACT INVARIANTS AT STAKE ({})", report.triggered_invariants.len());
        println!("────────────────────────────────────────────────────────────────────────────────");
        for inv in &report.triggered_invariants {
            println!("   • \x1b[1;33m{}\x1b[0m", inv);
        }
    }

    if !report.recommendations.is_empty() {
        println!("\n────────────────────────────────────────────────────────────────────────────────");
        println!("💡 AGENT PRE-COMMIT RECOMMENDATIONS");
        println!("────────────────────────────────────────────────────────────────────────────────");
        for rec in &report.recommendations {
            println!("   👉 {}", rec);
        }
    }

    println!("\n════════════════════════════════════════════════════════════════════════════════\n");
}

fn render_patch_simulation(sim: &PatchSimulationResult) {
    println!("\n╔══════════════════════════════════════════════════════════════════════════════╗");
    println!("║              🔬 STRATA PRE-FLIGHT PATCH SIMULATION REPORT                    ║");
    println!("╚══════════════════════════════════════════════════════════════════════════════╝\n");

    println!("📁 MODIFIED TARGETS:      {}", sim.modified_targets.join(", "));
    println!("👥 TOTAL IMPACTED NODES:  {}", sim.total_impacted_nodes);
    println!("💥 BREAKING CHANGE RISKS: {}", sim.breaking_risks_count);
    println!("🔥 HIGHEST PEAK RISK:     {:.0}%", sim.highest_risk_score * 100.0);

    if sim.safe_to_apply {
        println!("✅ VERDICT:               \x1b[1;32mSAFE TO APPLY (All contracts preserved)\x1b[0m\n");
    } else {
        println!("❌ VERDICT:               \x1b[1;31mRISKY (Violations or high breaking risks detected)\x1b[0m\n");
    }
}

fn format_node_kind(kind: CausalNodeKind) -> &'static str {
    match kind {
        CausalNodeKind::File => "\x1b[34m[File]\x1b[0m",
        CausalNodeKind::Module => "\x1b[35m[Module]\x1b[0m",
        CausalNodeKind::Struct => "\x1b[36m[Struct]\x1b[0m",
        CausalNodeKind::Enum => "\x1b[36m[Enum]\x1b[0m",
        CausalNodeKind::Trait => "\x1b[33m[Trait]\x1b[0m",
        CausalNodeKind::Function => "\x1b[32m[Fn]\x1b[0m",
        CausalNodeKind::Endpoint => "\x1b[1;32m[API]\x1b[0m",
        CausalNodeKind::DatabaseTable => "\x1b[1;35m[Table]\x1b[0m",
        CausalNodeKind::ConfigOption => "\x1b[90m[Config]\x1b[0m",
        CausalNodeKind::ContractInvariant => "\x1b[1;31m[Invariant]\x1b[0m",
    }
}
