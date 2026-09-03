use std::env;
use std::path::PathBuf;
use std::sync::Arc;
use anyhow::{Context, Result};
use clap::Args;
use strata_memory::{CodeAnchorEngine, SqliteStore};
use strata_reasoning::WorldModel;

#[derive(Debug, Args, Clone)]
pub struct ReconcileArgs {
    /// Workspace root directory to scan and reconcile (default: current directory)
    #[arg(long, default_value = ".")]
    pub workspace: PathBuf,

    /// Optional Git commit hash to record on updated anchors
    #[arg(long)]
    pub commit: Option<String>,

    /// Specific file or list of files to reconcile (comma-separated or multiple flags)
    #[arg(long, value_delimiter = ',')]
    pub files: Vec<String>,

    /// Output full reconciliation report in JSON format
    #[arg(long)]
    pub json: bool,
}

pub async fn run_reconcile(args: ReconcileArgs, store: Arc<SqliteStore>) -> Result<()> {
    let anchor_engine = CodeAnchorEngine::new();
    let world_model = WorldModel::new();

    let workspace_root = if args.workspace.is_relative() {
        let cwd = env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        cwd.join(&args.workspace)
    } else {
        args.workspace.clone()
    };

    // Index workspace files into causal world model for blast radius calculations
    let _ = world_model.index_workspace(&workspace_root).await;

    let report = if !args.files.is_empty() {
        // Reconcile specific files passed by user
        let mut file_tuples = Vec::new();
        let mut file_contents = Vec::new();

        for file_arg in &args.files {
            let file_path = PathBuf::from(file_arg);
            let abs_path = if file_path.is_relative() {
                workspace_root.join(&file_path)
            } else {
                file_path.clone()
            };

            if abs_path.exists() && abs_path.is_file() {
                let content = std::fs::read_to_string(&abs_path)
                    .with_context(|| format!("Failed to read file {}", abs_path.display()))?;
                let rel = abs_path
                    .strip_prefix(&workspace_root)
                    .unwrap_or(&abs_path)
                    .to_string_lossy()
                    .replace('\\', "/");
                file_contents.push(content);
                file_tuples.push((rel, file_contents.len() - 1));
            }
        }

        let slices: Vec<(&str, &str)> = file_tuples
            .iter()
            .map(|(p, idx)| (p.as_str(), file_contents[*idx].as_str()))
            .collect();

        anchor_engine
            .reconcile_workspace_on_commit(
                store.as_ref(),
                &slices,
                args.commit.as_deref(),
                Some(&world_model),
            )
            .await?
    } else {
        anchor_engine
            .reconcile_workspace_dir(
                store.as_ref(),
                &workspace_root,
                args.commit.as_deref(),
                Some(&world_model),
            )
            .await?
    };

    if args.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        render_reconciliation_cli(&report);
    }

    Ok(())
}

fn render_reconciliation_cli(report: &strata_memory::ast::ReconciliationReport) {
    println!("\n╔══════════════════════════════════════════════════════════════════════════════╗");
    println!("║       🔄 STRATA MERKLE-DRIVEN CODE ANCHOR RECONCILER (ON-COMMIT)            ║");
    println!("╚══════════════════════════════════════════════════════════════════════════════╝\n");

    println!("📊 FACTS SCANNED:        {}", report.total_facts_scanned);
    println!("🟢 ACTIVE INTACT FACTS:  \x1b[1;32m{}\x1b[0m", report.active_facts.len());
    println!("⚠️  STALE FACTS (MODIFIED): \x1b[1;31m{}\x1b[0m", report.stale_facts.len());
    println!("🟡 SUSPICIOUS FACTS:     \x1b[1;33m{}\x1b[0m (via Causal Blast Radius)", report.suspicious_facts.len());
    println!("🚚 RELOCATED ANCHORS:    \x1b[1;36m{}\x1b[0m (via Blake3 Content-Hash)", report.moved_anchors.len());
    println!("📝 UPDATED METADATA:     {}", report.updated_facts.len());

    if let Some(ref root) = report.merkle_root_after {
        println!("\n🌳 WORKSPACE MERKLE ROOT: \x1b[90m{}\x1b[0m", root);
    }

    if !report.stale_facts.is_empty() {
        println!("\n────────────────────────────────────────────────────────────────────────────────");
        println!("⚠️  STALE FACTS REQUIRING ATTENTION ({})", report.stale_facts.len());
        println!("────────────────────────────────────────────────────────────────────────────────");
        for id in &report.stale_facts {
            println!("   • Fact ID: \x1b[1;31m{}\x1b[0m (Code anchor invalidated, decay boosted)", id);
        }
    }

    if !report.suspicious_facts.is_empty() {
        println!("\n────────────────────────────────────────────────────────────────────────────────");
        println!("🟡 SUSPICIOUS DEPENDENT FACTS ({})", report.suspicious_facts.len());
        println!("────────────────────────────────────────────────────────────────────────────────");
        for id in &report.suspicious_facts {
            println!("   • Fact ID: \x1b[1;33m{}\x1b[0m (Impacted by upstream code changes in blast radius)", id);
        }
    }

    if !report.moved_anchors.is_empty() {
        println!("\n────────────────────────────────────────────────────────────────────────────────");
        println!("🚚 RELOCATED ANCHORS PRESERVED ({})", report.moved_anchors.len());
        println!("────────────────────────────────────────────────────────────────────────────────");
        for id in &report.moved_anchors {
            println!("   • Fact ID: \x1b[1;36m{}\x1b[0m (Rename/move tolerated via Blake3 exact body match)", id);
        }
    }

    if report.stale_facts.is_empty() && report.suspicious_facts.is_empty() {
        println!("\n✅ All semantic facts and code anchors are in sync with workspace code.");
    } else {
        println!("\n💡 Tip: Run `strata prune` to enforce decay pruning on stale facts, or re-verify suspicious facts.");
    }

    println!("\n════════════════════════════════════════════════════════════════════════════════\n");
}
