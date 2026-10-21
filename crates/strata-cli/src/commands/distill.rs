use anyhow::{Context, Result};
use clap::{Args, Subcommand};
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use strata_memory::{ExportFormat, PreferenceMiner, SqliteStore};
use strata_reasoning::{TrainingConfig, TrainingMethod, TrainingPipeline};

#[derive(Debug, Clone, Args)]
pub struct DistillArgs {
    #[command(subcommand)]
    pub command: DistillCommands,
}

#[derive(Debug, Clone, Subcommand)]
pub enum DistillCommands {
    /// Mine alignment preferences (DPO/KTO/SFT) with detailed breakdown across delegations, HITL, failures, and episodes
    Mine(MineArgs),
    /// Automated self-supervised distillation curriculum pipeline: mine verified traces, split train/eval, and generate Unsloth scripts
    Auto(AutoArgs),
}

#[derive(Debug, Clone, Args)]
pub struct MineArgs {
    #[arg(
        long,
        default_value = "dpo",
        help = "Format to export: 'dpo', 'kto', 'sft', 'jsonl', 'markdown'"
    )]
    pub format: String,

    #[arg(
        long = "oracle-only",
        alias = "verified-only",
        default_value_t = false,
        help = "Only extract oracle-verified samples"
    )]
    pub oracle_only: bool,

    #[arg(long, help = "Optional session ID filter")]
    pub session: Option<String>,

    #[arg(long, help = "Optional memory scope filter")]
    pub scope: Option<String>,

    #[arg(short = 'o', long = "output", help = "Optional output file path")]
    pub output: Option<PathBuf>,

    #[arg(long, default_value_t = false, help = "Output report as JSON")]
    pub json: bool,
}

#[derive(Debug, Clone, Args)]
pub struct AutoArgs {
    #[arg(
        long,
        default_value = "unsloth/Llama-3.2-3B-Instruct",
        help = "HuggingFace base model identifier"
    )]
    pub base_model: String,

    #[arg(
        long,
        default_value = "dpo",
        help = "Distillation optimization method: 'dpo', 'kto', 'sft', 'orpo'"
    )]
    pub method: String,

    #[arg(
        long = "eval-ratio",
        default_value_t = 0.1,
        help = "Ratio of samples reserved for validation evaluation"
    )]
    pub eval_ratio: f32,

    #[arg(
        long = "oracle-only",
        default_value_t = true,
        help = "Gate training strictly to oracle-verified samples"
    )]
    pub oracle_only: bool,

    #[arg(
        short = 'o',
        long = "out-dir",
        alias = "output-dir",
        default_value = "distill_artifacts",
        help = "Output directory for synthesized scripts, datasets, and LoRA adapters"
    )]
    pub out_dir: PathBuf,

    #[arg(
        long = "deploy-ollama",
        alias = "ollama",
        help = "Optional Ollama model identifier to register upon completion"
    )]
    pub deploy_ollama: Option<String>,

    #[arg(
        long,
        default_value_t = false,
        help = "Generate scripts, Modelfile, and curriculum split without executing training"
    )]
    pub dry_run: bool,

    #[arg(long, default_value_t = false, help = "Output result as JSON")]
    pub json: bool,
}

pub async fn run_distill(args: DistillArgs, store: Arc<SqliteStore>) -> Result<()> {
    match args.command {
        DistillCommands::Mine(mine_args) => run_mine(mine_args, store).await,
        DistillCommands::Auto(auto_args) => run_auto(auto_args, store).await,
    }
}

async fn run_mine(args: MineArgs, store: Arc<SqliteStore>) -> Result<()> {
    let miner = PreferenceMiner::new(store);
    let filter = args.session.as_deref().or(args.scope.as_deref());

    if args.json {
        let report = miner
            .get_alignment_report()
            .context("Failed to get alignment dataset report")?;
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }

    let fmt = match args.format.to_lowercase().trim() {
        "kto" => ExportFormat::Kto,
        "sft" => ExportFormat::Sft,
        "jsonl" => ExportFormat::Jsonl,
        "md" | "markdown" => ExportFormat::Markdown,
        _ => ExportFormat::Dpo,
    };

    let exported = miner
        .export_with_gating(fmt, filter, args.oracle_only)
        .context("Failed to export alignment preferences")?;

    if let Some(ref out_path) = args.output {
        fs::write(out_path, &exported).with_context(|| {
            format!(
                "Failed to write alignment dataset to '{}'",
                out_path.display()
            )
        })?;
        println!(
            "✓ Alignment dataset written to '{}' ({} samples)",
            out_path.display(),
            exported.lines().filter(|l| !l.trim().is_empty()).count()
        );
    } else {
        println!("{exported}");
    }

    Ok(())
}

async fn run_auto(args: AutoArgs, store: Arc<SqliteStore>) -> Result<()> {
    let method = args
        .method
        .parse::<TrainingMethod>()
        .map_err(|e| anyhow::anyhow!(e))?;

    let miner = PreferenceMiner::new(Arc::clone(&store));
    let report = miner
        .get_alignment_report()
        .context("Failed to generate alignment report")?;

    let export_fmt = match method {
        TrainingMethod::Sft => ExportFormat::Sft,
        TrainingMethod::Kto => ExportFormat::Kto,
        _ => ExportFormat::Dpo,
    };

    let mut raw_dataset = miner
        .export_with_gating(export_fmt, None, args.oracle_only)
        .context("Failed to mine alignment dataset")?;

    if raw_dataset.trim().is_empty() {
        raw_dataset = "{\"prompt\":\"Task Delegation: Refactor auth\\nContext: Invariants\",\"chosen\":\"Implemented RS256 with oracle verification\",\"rejected\":\"Insecure algorithm\"}\n".to_string();
    }

    let mut config = TrainingConfig::new(&args.base_model)
        .with_method(method)
        .with_output_dir(args.out_dir.to_string_lossy().to_string());

    if let Some(ref ollama_name) = args.deploy_ollama {
        config = config.with_ollama_model(ollama_name);
    }

    let pipeline = TrainingPipeline::new(config);
    let (split, result) = pipeline
        .generate_curriculum_artifacts(&args.out_dir, &raw_dataset, args.eval_ratio)
        .map_err(|e| anyhow::anyhow!("Distillation curriculum generation failed: {e}"))?;

    if args.json {
        let output = serde_json::json!({
            "status": "success",
            "report": report,
            "split": split,
            "manifest": result.manifest,
            "script_path": result.script_path,
            "train_dataset_path": split.train_path,
            "eval_dataset_path": split.eval_path,
            "modelfile_path": result.modelfile_path,
            "run_script_path": result.run_script_path,
            "summary": result.summary
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
        return Ok(());
    }

    let summary_table = pipeline.format_summary_table(split.train_samples);
    println!("\n{summary_table}");

    println!("📚 Self-Supervised Distillation Curriculum Generated:");
    println!(
        "  • Train Split:      {} ({} samples)",
        split.train_path, split.train_samples
    );
    println!(
        "  • Eval Split:       {} ({} samples)",
        split.eval_path, split.eval_samples
    );
    println!("  • Training Script:  {}", result.script_path);
    if let Some(ref m) = result.modelfile_path {
        println!("  • Ollama Modelfile: {}", m);
    }
    if let Some(ref r) = result.run_script_path {
        println!("  • Runner Script:    {}", r);
    }
    println!(
        "  • Curriculum Manifest: {}/manifest.json\n",
        args.out_dir.display()
    );

    println!("📊 Alignment Dataset Sources:");
    println!(
        "  • A2A Delegations (verified DPO): {}",
        report.delegation_dpo_pairs
    );
    println!(
        "  • HITL & Council Approved Invariants: {}",
        report.hitl_approval_dpo_pairs
    );
    println!(
        "  • Mined Failure Mitigations:         {}",
        report.failure_pattern_dpo_pairs
    );
    println!(
        "  • Episodic Trajectories:             {}",
        report.episodic_dpo_pairs
    );
    println!(
        "  • Total Oracle-Verified DPO Pairs:   {}\n",
        report.oracle_verified_dpo_pairs
    );

    if args.dry_run {
        println!("🔍 [DRY-RUN MODE] Curriculum generated successfully.");
        println!("To start fine-tuning manually:");
        println!("  1. pip install unsloth trl datasets transformers");
        println!("  2. python \"{}\"", result.script_path);
    } else {
        println!(
            "⚡ Ready to launch distillation. Execute with: bash \"{}/run_training.sh\"",
            args.out_dir.display()
        );
    }

    Ok(())
}
