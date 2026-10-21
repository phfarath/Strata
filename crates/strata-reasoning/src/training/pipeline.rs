use chrono::Utc;
use serde::{Deserialize, Serialize};
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

/// Summary of a curriculum dataset split into training and evaluation sets.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CurriculumSplitResult {
    pub total_raw_samples: usize,
    pub unique_samples: usize,
    pub train_samples: usize,
    pub eval_samples: usize,
    pub eval_ratio: f32,
    pub train_path: String,
    pub eval_path: String,
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
        self.generate_artifacts_with_eval(output_dir, dataset_content, samples_count, None)
    }

    /// Synthesize all training artifacts including an optional evaluation dataset.
    pub fn generate_artifacts_with_eval(
        &self,
        output_dir: &Path,
        dataset_content: Option<&str>,
        samples_count: usize,
        eval_dataset_path: Option<&Path>,
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
        let eval_path_str = eval_dataset_path.map(|p| p.to_string_lossy().to_string());
        let script_content = generate_unsloth_training_script(
            &self.config,
            &dataset_path.to_string_lossy(),
            eval_path_str.as_deref(),
        );
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

    /// Extract a semantic deduplication key from a JSONL sample record.
    pub fn extract_semantic_key(line: &str) -> String {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(line) {
            if let Some(obj) = v.as_object() {
                if obj.contains_key("prompt")
                    && (obj.contains_key("chosen") || obj.contains_key("rejected"))
                {
                    let p = obj.get("prompt").and_then(|x| x.as_str()).unwrap_or("");
                    let c = obj.get("chosen").and_then(|x| x.as_str()).unwrap_or("");
                    let r = obj.get("rejected").and_then(|x| x.as_str()).unwrap_or("");
                    return format!("dpo::{p}::{c}::{r}");
                }
                if obj.contains_key("prompt")
                    && (obj.contains_key("completion") || obj.contains_key("label"))
                {
                    let p = obj.get("prompt").and_then(|x| x.as_str()).unwrap_or("");
                    let c = obj.get("completion").and_then(|x| x.as_str()).unwrap_or("");
                    let l = obj.get("label").and_then(|x| x.as_bool()).unwrap_or(true);
                    return format!("kto::{p}::{c}::{l}");
                }
                if obj.contains_key("instruction") || obj.contains_key("output") {
                    let inst = obj
                        .get("instruction")
                        .and_then(|x| x.as_str())
                        .unwrap_or("");
                    let inp = obj.get("input").and_then(|x| x.as_str()).unwrap_or("");
                    let out = obj.get("output").and_then(|x| x.as_str()).unwrap_or("");
                    return format!("sft::{inst}::{inp}::{out}");
                }
            }
        }
        line.to_string()
    }

    /// Split a raw JSONL dataset into deduplicated train and eval splits deterministically.
    pub fn split_dataset_curriculum(
        raw_jsonl: &str,
        eval_ratio: f32,
        seed: u64,
    ) -> (Vec<String>, Vec<String>) {
        let mut seen = std::collections::HashSet::new();
        let mut unique = Vec::new();

        for line in raw_jsonl.lines() {
            let trimmed = line.trim();
            if !trimmed.is_empty() {
                let key = Self::extract_semantic_key(trimmed);
                if seen.insert(key) {
                    unique.push(trimmed.to_string());
                }
            }
        }

        if unique.is_empty() {
            return (Vec::new(), Vec::new());
        }

        // Deterministic Fisher-Yates shuffle using simple 64-bit LCG
        let mut rng_state = if seed == 0 { 42 } else { seed };
        let mut shuffled = unique;
        let n = shuffled.len();
        for i in (1..n).rev() {
            rng_state = rng_state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let j = (rng_state >> 33) as usize % (i + 1);
            shuffled.swap(i, j);
        }

        let clamped_ratio = eval_ratio.clamp(0.0, 0.5);
        let mut eval_count = ((shuffled.len() as f32) * clamped_ratio).round() as usize;
        if eval_count == 0 && clamped_ratio > 0.0 && shuffled.len() > 1 {
            eval_count = 1;
        }
        if eval_count >= shuffled.len() {
            eval_count = shuffled.len().saturating_sub(1);
        }

        let train_count = shuffled.len() - eval_count;
        let eval_samples = shuffled.split_off(train_count);
        let train_samples = shuffled;

        (train_samples, eval_samples)
    }

    /// Synthesize self-supervised distillation curriculum artifacts:
    /// splits raw dataset into train.jsonl and eval.jsonl, and generates training scripts.
    pub fn generate_curriculum_artifacts(
        &self,
        output_dir: &Path,
        raw_dataset: &str,
        eval_ratio: f32,
    ) -> Result<(CurriculumSplitResult, TrainingResult), StrataError> {
        self.validate_config()?;

        fs::create_dir_all(output_dir)
            .map_err(|e| StrataError::Io(format!("Failed to create output dir: {e}")))?;

        let raw_lines_count = raw_dataset.lines().filter(|l| !l.trim().is_empty()).count();
        let (train_lines, eval_lines) =
            Self::split_dataset_curriculum(raw_dataset, eval_ratio, self.config.seed);

        let train_path = output_dir.join("train.jsonl");
        let train_content = train_lines.join("\n") + if train_lines.is_empty() { "" } else { "\n" };
        fs::write(&train_path, &train_content)
            .map_err(|e| StrataError::Io(format!("Failed to write train.jsonl: {e}")))?;

        let eval_path = output_dir.join("eval.jsonl");
        let eval_content = eval_lines.join("\n") + if eval_lines.is_empty() { "" } else { "\n" };
        fs::write(&eval_path, &eval_content)
            .map_err(|e| StrataError::Io(format!("Failed to write eval.jsonl: {e}")))?;

        let split_result = CurriculumSplitResult {
            total_raw_samples: raw_lines_count,
            unique_samples: train_lines.len() + eval_lines.len(),
            train_samples: train_lines.len(),
            eval_samples: eval_lines.len(),
            eval_ratio,
            train_path: train_path.to_string_lossy().to_string(),
            eval_path: eval_path.to_string_lossy().to_string(),
        };

        let training_result = self.generate_artifacts_with_eval(
            output_dir,
            Some(&train_content),
            train_lines.len(),
            Some(&eval_path),
        )?;

        Ok((split_result, training_result))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::training::types::{TrainingConfig, TrainingMethod};

    #[test]
    fn test_split_dataset_curriculum_semantic_deduplication() {
        let sample1 = r#"{"id":"00000000-0000-0000-0000-000000000001","prompt":"Delegation task auth","chosen":"Use RS256 with JWT","rejected":"Use plain text auth","created_at":"2026-10-01T00:00:00Z"}"#;
        let sample2 = r#"{"id":"ffffffff-ffff-ffff-ffff-ffffffffffff","prompt":"Delegation task auth","chosen":"Use RS256 with JWT","rejected":"Use plain text auth","created_at":"2026-10-02T12:00:00Z"}"#;
        let sample3 = r#"{"id":"33333333-3333-3333-3333-333333333333","prompt":"Delegation task storage","chosen":"Use SQLite WAL mode","rejected":"No lock backoff","created_at":"2026-10-03T00:00:00Z"}"#;

        let raw = format!("{}\n{}\n{}\n", sample1, sample2, sample3);

        let (train, eval) = TrainingPipeline::split_dataset_curriculum(&raw, 0.5, 42);
        assert_eq!(
            train.len() + eval.len(),
            2,
            "Duplicate semantic record must be dropped even with different UUID/timestamp"
        );
    }

    #[test]
    fn test_generate_curriculum_artifacts_wires_eval() {
        let temp_dir =
            std::env::temp_dir().join(format!("strata_test_pipeline_{}", uuid::Uuid::new_v4()));

        let mut raw = String::new();
        for i in 0..10 {
            raw.push_str(&format!(
                "{{\"prompt\":\"Task {}\",\"chosen\":\"c {}\",\"rejected\":\"r {}\"}}\n",
                i, i, i
            ));
        }

        let config = TrainingConfig::new("unsloth/Llama-3.2-3B-Instruct")
            .with_method(TrainingMethod::Dpo)
            .with_output_dir(temp_dir.to_string_lossy().to_string());

        let pipeline = TrainingPipeline::new(config);
        let (split, result) = pipeline
            .generate_curriculum_artifacts(&temp_dir, &raw, 0.2)
            .expect("Curriculum artifact generation should succeed");

        assert_eq!(split.total_raw_samples, 10);
        assert_eq!(split.train_samples, 8);
        assert_eq!(split.eval_samples, 2);

        let script =
            std::fs::read_to_string(&result.script_path).expect("train_lora.py must be readable");
        assert!(
            script.contains("eval_dataset = eval_dataset"),
            "Script should wire eval dataset"
        );
        assert!(
            script.contains("eval_strategy = \"steps\""),
            "Script should enable evaluation steps"
        );

        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}
