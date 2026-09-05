use anyhow::{Context, Result};
use clap::Args;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use strata_memory::{ExportFormat, PreferenceMiner, SqliteStore};
use strata_reasoning::{QuantizationType, TrainingConfig, TrainingMethod, TrainingPipeline};

#[derive(Debug, Clone, Args)]
pub struct TrainArgs {
    #[arg(
        long,
        default_value = "unsloth/Llama-3.2-3B-Instruct",
        help = "HuggingFace base model identifier (e.g. 'unsloth/Llama-3.2-3B-Instruct', 'unsloth/Qwen2.5-Coder-7B-Instruct')"
    )]
    pub base_model: String,

    #[arg(
        long,
        default_value = "dpo",
        help = "Fine-tuning optimization method: 'dpo', 'sft', 'orpo', 'kto'"
    )]
    pub method: String,

    #[arg(
        long,
        default_value = "4bit",
        help = "Quantization format for model loading: '4bit', '8bit', '16bit', 'none'"
    )]
    pub quantization: String,

    #[arg(long, default_value_t = 16, help = "LoRA rank dimension (r)")]
    pub lora_r: u32,

    #[arg(long, default_value_t = 32, help = "LoRA alpha scaling parameter")]
    pub lora_alpha: u32,

    #[arg(long, default_value_t = 0.0, help = "LoRA dropout probability")]
    pub lora_dropout: f32,

    #[arg(
        long = "lr",
        alias = "learning-rate",
        default_value_t = 5e-5,
        help = "AdamW optimizer learning rate"
    )]
    pub learning_rate: f64,

    #[arg(long, default_value_t = 2, help = "Per-device training batch size")]
    pub batch_size: usize,

    #[arg(
        long = "grad-accum",
        alias = "gradient-accumulation-steps",
        default_value_t = 4,
        help = "Gradient accumulation steps"
    )]
    pub gradient_accumulation_steps: usize,

    #[arg(long, default_value_t = 60, help = "Total training steps")]
    pub max_steps: usize,

    #[arg(
        long,
        default_value_t = 2048,
        help = "Maximum sequence token context length"
    )]
    pub max_seq_length: usize,

    #[arg(
        short = 'o',
        long = "out-dir",
        alias = "output-dir",
        default_value = "training_artifacts",
        help = "Target output directory for synthesized scripts, datasets, and LoRA adapters"
    )]
    pub out_dir: PathBuf,

    #[arg(
        long = "deploy-ollama",
        alias = "ollama",
        help = "Optional Ollama model identifier to register upon training completion (e.g. 'strata-custom-coder')"
    )]
    pub deploy_ollama: Option<String>,

    #[arg(
        long,
        help = "Path to custom JSONL dataset file (if omitted, dataset is mined from Strata continuous memory)"
    )]
    pub dataset: Option<PathBuf>,

    #[arg(long, help = "Optional memory scope filter for dataset mining")]
    pub scope: Option<String>,

    #[arg(long, help = "Optional session ID filter for dataset mining")]
    pub session: Option<String>,

    #[arg(
        long,
        default_value_t = false,
        help = "Generate scripts, Modelfile, and dataset manifest without executing Python training"
    )]
    pub dry_run: bool,

    #[arg(long, default_value_t = false, help = "Output report as raw JSON")]
    pub json: bool,
}

pub async fn run_train(args: TrainArgs, store: Arc<SqliteStore>) -> Result<()> {
    let method = args
        .method
        .parse::<TrainingMethod>()
        .map_err(|e| anyhow::anyhow!(e))?;

    let quantization = args
        .quantization
        .parse::<QuantizationType>()
        .map_err(|e| anyhow::anyhow!(e))?;

    let filter = args.session.as_deref().or(args.scope.as_deref());

    // 1. Prepare Dataset
    let (dataset_content, sample_count) = if let Some(ref custom_path) = args.dataset {
        let content = fs::read_to_string(custom_path).with_context(|| {
            format!(
                "Failed to read custom dataset at '{}'",
                custom_path.display()
            )
        })?;
        let count = content.lines().filter(|l| !l.trim().is_empty()).count();
        (Some(content), count)
    } else {
        let miner = PreferenceMiner::new(store);
        let export_fmt = match method {
            TrainingMethod::Sft => ExportFormat::Sft,
            TrainingMethod::Kto => ExportFormat::Kto,
            _ => ExportFormat::Dpo,
        };
        let mined = miner
            .export(export_fmt, filter)
            .context("Failed to mine alignment dataset from Strata continuous memory")?;
        let count = mined.lines().filter(|l| !l.trim().is_empty()).count();
        (Some(mined), count)
    };

    // 2. Build TrainingConfig
    let mut config = TrainingConfig::new(&args.base_model)
        .with_method(method)
        .with_quantization(quantization)
        .with_lora(args.lora_r, args.lora_alpha, args.lora_dropout)
        .with_learning_rate(args.learning_rate)
        .with_batch_size(args.batch_size, args.gradient_accumulation_steps)
        .with_max_steps(args.max_steps)
        .with_max_seq_length(args.max_seq_length)
        .with_output_dir(args.out_dir.to_string_lossy().to_string());

    if let Some(ref ollama_name) = args.deploy_ollama {
        config = config.with_ollama_model(ollama_name);
    }

    // 3. Instantiate Pipeline & Generate Artifacts
    let pipeline = TrainingPipeline::new(config);
    let result = pipeline
        .generate_artifacts(&args.out_dir, dataset_content.as_deref(), sample_count)
        .map_err(|e| anyhow::anyhow!("Pipeline artifact generation failed: {e}"))?;

    // 4. Handle JSON or Interactive Output
    if args.json {
        println!("{}", serde_json::to_string_pretty(&result)?);
        return Ok(());
    }

    let summary_table = pipeline.format_summary_table(sample_count);
    println!("\n{summary_table}");

    println!("📁 Generated Training Artifacts:");
    println!("  • Python Script:  {}", result.script_path);
    println!(
        "  • Dataset JSONL:  {} ({} samples)",
        result.dataset_path, sample_count
    );
    if let Some(ref m_path) = result.modelfile_path {
        println!("  • Ollama Modelfile: {}", m_path);
    }
    if let Some(ref r_path) = result.run_script_path {
        println!("  • Runner Script:  {}", r_path);
    }
    println!(
        "  • Run Manifest:   {}/manifest.json\n",
        args.out_dir.display()
    );

    if args.dry_run {
        println!("🔍 [DRY-RUN MODE] Artifacts synthesized successfully.");
        println!("To start local fine-tuning manually:");
        println!("  1. pip install unsloth trl datasets transformers");
        println!("  2. python \"{}\"", result.script_path);
        if let Some(ref ollama_name) = args.deploy_ollama {
            println!(
                "  3. ollama create \"{}\" -f \"{}/Modelfile\"",
                ollama_name,
                args.out_dir.display()
            );
            println!("  4. ollama run \"{}\"", ollama_name);
        }
    } else {
        println!(
            "⚡ Ready to launch. Execute with: bash \"{}/run_training.sh\"",
            args.out_dir.display()
        );
    }

    Ok(())
}
