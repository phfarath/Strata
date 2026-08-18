use std::path::PathBuf;
use std::sync::Arc;
use anyhow::{Context, Result};
use clap::Args;
use strata_memory::{MultiHostCompiler, SqliteStore};

#[derive(Debug, Clone, Args)]
pub struct SyncHostsArgs {
    #[arg(long, default_value = "all", help = "Target hosts: 'cursor', 'claude', 'codex', 'gemini', or comma-separated list")]
    pub target: String,

    #[arg(long, default_value_t = 1000, help = "Maximum token budget for compiled context")]
    pub budget: usize,

    #[arg(long, default_value = ".", help = "Target workspace directory")]
    pub workspace: PathBuf,

    #[arg(long, help = "Output report as JSON")]
    pub json: bool,
}

pub async fn run_sync_hosts(args: SyncHostsArgs, store: Arc<SqliteStore>) -> Result<()> {
    let targets: Vec<&str> = if args.target.eq_ignore_ascii_case("all") {
        vec!["cursor", "claude", "codex", "gemini"]
    } else {
        args.target.split(',').map(|s| s.trim()).filter(|s| !s.is_empty()).collect()
    };

    let compiler = MultiHostCompiler::new(store);
    let report = compiler
        .compile_workspace(&args.workspace, &targets, args.budget)
        .context("Failed to compile multi-host instructions")?;

    if args.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!("\n🔄 [Strata Multi-Host Instruction Sync]");
        println!("═════════════════════════════════════════");
        println!("Workspace:     {}", args.workspace.display());
        println!("Token Budget:  {} tokens", args.budget);
        println!("Compiled:      ~{} tokens\n", report.total_tokens);

        for target in &report.target_hosts {
            println!(
                "  ✓ [{}] -> {} (~{} tokens)",
                target.host,
                target.target_file.display(),
                target.tokens_compiled
            );
        }
        println!("\n✨ Host instructions updated successfully within token budget.\n");
    }

    Ok(())
}
