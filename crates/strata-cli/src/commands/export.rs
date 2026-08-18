use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use anyhow::{Context, Result};
use clap::Args;
use strata_memory::{ExportFormat, PreferenceMiner, SqliteStore};

#[derive(Debug, Clone, Args)]
pub struct ExportArgs {
    #[arg(long, default_value = "dpo", help = "Export format: dpo, kto, sft, markdown, jsonl")]
    pub format: String,

    #[arg(short, long, help = "Output destination file path (default: stdout)")]
    pub out: Option<PathBuf>,

    #[arg(long, help = "Optional scope filter: 'global', 'project:<name>', 'session:<id>'")]
    pub scope: Option<String>,

    #[arg(long, help = "Optional session ID filter")]
    pub session: Option<String>,
}

pub async fn run_export(args: ExportArgs, store: Arc<SqliteStore>) -> Result<()> {
    let format = args.format.parse::<ExportFormat>().map_err(|e| anyhow::anyhow!(e))?;
    let filter = args.session.as_deref().or(args.scope.as_deref());

    let miner = PreferenceMiner::new(store);
    let output = miner.export(format, filter)
        .context("Failed to mine and export alignment preferences")?;

    if let Some(out_path) = args.out {
        if let Some(parent) = out_path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create directory '{}'", parent.display()))?;
        }
        fs::write(&out_path, &output)
            .with_context(|| format!("Failed to write export output to '{}'", out_path.display()))?;
        eprintln!("✓ Mined dataset exported successfully to '{}' (format: {:?})", out_path.display(), format);
    } else {
        print!("{output}");
    }

    Ok(())
}
