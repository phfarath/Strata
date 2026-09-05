use chrono::Utc;
use std::fs;
use std::path::{Path, PathBuf};
use strata_core::errors::StrataError;
use uuid::Uuid;

use super::generator::{
    generate_ollama_modelfile, generate_run_script, generate_unsloth_training_script,
};
use super::types::{TrainingConfig, TrainingManifest, TrainingResult};

/// End-to-end orchestrator for dataset formatting, Unsloth Python script synthesis,
/// Ollama Modelfile compilation, and artifact management.
pub struct TrainingPipeline {
    config: TrainingConfig,
}

impl TrainingPipeline {
    /// Create a new `TrainingPipeline` with the given configuration.
    pub fn new(config: TrainingConfig) -> Self {
        Self { config }
    }

    /// Access the underlying training configuration.
    pub fn config(&self) -> &TrainingConfig {
        &self.config
    }

    /// Validate configuration invariants before starting artifact synthesis.
    pub fn validate_config(&self) -> Result<(), StrataError> {
        self.config.validate()
    }

    /// Format an ASCII summary table detailing all hyperparameters and target deployment.
    pub fn format_summary_table(&self, samples_count: usize) -> String {
        let mut table = String::new();
        table.push_str(
            "┌───────────────────────────────────┬────────────────────────────────────────────┐\n",
        );
        table.push_str(
            "│ STRATA LORA FINE-TUNING PIPELINE  │ CONFIGURATION PARAMETERS                   │\n",
        );
        table.push_str(
            "├───────────────────────────────────┼────────────────────────────────────────────┤\n",
        );
        table.push_str(&format!(
            "│ Base Foundation Model             │ {:<42} │\n",
            self.config.base_model
        ));
        table.push_str(&format!(
            "│ Training Optimization Method      │ {:<42} │\n",
            self.config.method.to_string().to_uppercase()
        ));
        table.push_str(&format!(
            "│ Model Quantization                │ {:<42} │\n",
            self.config.quantization.to_string()
        ));
        table.push_str(&format!(
            "│ LoRA Hyperparameters              │ r={}, alpha={}, dropout={:<18} │\n",
            self.config.lora_r, self.config.lora_alpha, self.config.lora_dropout
        ));
        table.push_str(&format!(
            "│ Target Linear Modules             │ {:<42} │\n",
            format!("{} modules", self.config.target_modules.len())
        ));
        table.push_str(&format!(
            "│ Max Sequence Context              │ {:<42} │\n",
            format!("{} tokens", self.config.max_seq_length)
        ));
        table.push_str(&format!(
            "│ Batch Size & Grad Accum           │ batch={}, accum={} (effective: {}){:>10} │\n",
            self.config.batch_size,
            self.config.gradient_accumulation_steps,
            self.config.batch_size * self.config.gradient_accumulation_steps,
            ""
        ));
        table.push_str(&format!(
            "│ Learning Rate                     │ {:<42} │\n",
            self.config.learning_rate
        ));
        table.push_str(&format!(
            "│ Max Training Steps                │ {:<42} │\n",
            self.config.max_steps
        ));
        table.push_str(&format!(
            "│ Mined Alignment Samples           │ {:<42} │\n",
            format!("{} samples", samples_count)
        ));
        table.push_str(&format!(
            "│ GGUF Ollama Export                │ {:<42} │\n",
            if self.config.export_gguf {
                format!("Enabled ({})", self.config.gguf_quantization)
            } else {
                "Disabled".to_string()
            }
        ));
        table.push_str(&format!(
            "│ Target Ollama Model               │ {:<42} │\n",
            self.config.ollama_model_name.as_deref().unwrap_or("None")
        ));
        table.push_str(&format!(
            "│ Artifact Output Directory         │ {:<42} │\n",
            self.config.output_dir
        ));
        table.push_str(
            "└───────────────────────────────────┴────────────────────────────────────────────┘\n",
        );
        table
    }

    /// Synthesize all training artifacts into the specified output directory.
    pub fn generate_artifacts(
        &self,
        output_dir: &Path,
        dataset_content: Option<&str>,
        samples_count: usize,
    ) -> Result<TrainingResult, StrataError> {
        self.validate_config()?;

        fs::create_dir_all(output_dir)
            .map_err(|e| StrataError::Io(format!("Failed to create output dir: {e}")))?;

        // 1. Write or locate dataset
        let dataset_path: PathBuf = if let Some(content) = dataset_content {
            let p = output_dir.join("dataset.jsonl");
            fs::write(&p, content)
                .map_err(|e| StrataError::Io(format!("Failed to write dataset.jsonl: {e}")))?;
            p
        } else if let Some(ref custom_p) = self.config.dataset_path {
            PathBuf::from(custom_p)
        } else {
            let p = output_dir.join("dataset.jsonl");
            if !p.exists() {
                fs::write(&p, "[]\n").map_err(|e| {
                    StrataError::Io(format!("Failed to initialize empty dataset: {e}"))
                })?;
            }
            p
        };

        // 2. Synthesize Unsloth Python script
        let script_content =
            generate_unsloth_training_script(&self.config, &dataset_path.to_string_lossy());
        let script_path = output_dir.join("train_lora.py");
        fs::write(&script_path, &script_content)
            .map_err(|e| StrataError::Io(format!("Failed to write train_lora.py: {e}")))?;

        // 3. Synthesize Ollama Modelfile
        let adapter_target = if self.config.export_gguf {
            format!("{}/gguf", self.config.output_dir)
        } else {
            self.config.output_dir.clone()
        };
        let modelfile_content = generate_ollama_modelfile(&self.config, &adapter_target);
        let modelfile_path = output_dir.join("Modelfile");
        fs::write(&modelfile_path, &modelfile_content)
            .map_err(|e| StrataError::Io(format!("Failed to write Modelfile: {e}")))?;

        // 4. Synthesize run_training.sh
        let run_script_content = generate_run_script(
            &self.config,
            &script_path.to_string_lossy(),
            &output_dir.to_string_lossy(),
        );
        let run_script_path = output_dir.join("run_training.sh");
        fs::write(&run_script_path, &run_script_content)
            .map_err(|e| StrataError::Io(format!("Failed to write run_training.sh: {e}")))?;

        // 5. Generate Manifest
        let manifest_id = Uuid::new_v4().to_string();
        let manifest = TrainingManifest {
            id: manifest_id,
            created_at: Utc::now(),
            base_model: self.config.base_model.clone(),
            method: self.config.method,
            total_samples: samples_count,
            config: self.config.clone(),
            script_path: script_path.to_string_lossy().to_string(),
            dataset_path: dataset_path.to_string_lossy().to_string(),
            modelfile_path: Some(modelfile_path.to_string_lossy().to_string()),
            run_script_path: Some(run_script_path.to_string_lossy().to_string()),
            ollama_model_name: self.config.ollama_model_name.clone(),
            status: "ready".to_string(),
        };

        let manifest_json = serde_json::to_string_pretty(&manifest)
            .map_err(|e| StrataError::Serialization(e.to_string()))?;
        let manifest_path = output_dir.join("manifest.json");
        fs::write(&manifest_path, manifest_json)
            .map_err(|e| StrataError::Io(format!("Failed to write manifest.json: {e}")))?;

        let summary = format!(
            "Successfully synthesized LoRA fine-tuning artifacts in '{}':\n  • Python Script: {}\n  • Modelfile:     {}\n  • Run Script:    {}\n  • Manifest:      {}\n  • Dataset:       {} ({} samples)",
            output_dir.display(),
            script_path.file_name().unwrap_or_default().to_string_lossy(),
            modelfile_path.file_name().unwrap_or_default().to_string_lossy(),
            run_script_path.file_name().unwrap_or_default().to_string_lossy(),
            manifest_path.file_name().unwrap_or_default().to_string_lossy(),
            dataset_path.file_name().unwrap_or_default().to_string_lossy(),
            samples_count,
        );

        Ok(TrainingResult {
            success: true,
            manifest,
            script_path: script_path.to_string_lossy().to_string(),
            dataset_path: dataset_path.to_string_lossy().to_string(),
            modelfile_path: Some(modelfile_path.to_string_lossy().to_string()),
            run_script_path: Some(run_script_path.to_string_lossy().to_string()),
            summary,
            preview_code: Some(script_content),
        })
    }
}
