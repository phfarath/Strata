use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;
use strata_core::errors::StrataError;

/// Fine-tuning optimization method.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TrainingMethod {
    #[default]
    Dpo,
    Sft,
    Orpo,
    Kto,
}

impl fmt::Display for TrainingMethod {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Dpo => write!(f, "dpo"),
            Self::Sft => write!(f, "sft"),
            Self::Orpo => write!(f, "orpo"),
            Self::Kto => write!(f, "kto"),
        }
    }
}

impl FromStr for TrainingMethod {
    type Err = StrataError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().trim() {
            "dpo" => Ok(Self::Dpo),
            "sft" => Ok(Self::Sft),
            "orpo" => Ok(Self::Orpo),
            "kto" => Ok(Self::Kto),
            other => Err(StrataError::ValidationError(format!(
                "Unknown training method '{}'. Supported: dpo, sft, orpo, kto",
                other
            ))),
        }
    }
}

/// Quantization format for base model loading and LoRA training.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum QuantizationType {
    #[default]
    Bits4,
    Bits8,
    Bits16,
    None,
}

impl fmt::Display for QuantizationType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Bits4 => write!(f, "4bit"),
            Self::Bits8 => write!(f, "8bit"),
            Self::Bits16 => write!(f, "16bit"),
            Self::None => write!(f, "none"),
        }
    }
}

impl FromStr for QuantizationType {
    type Err = StrataError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().trim() {
            "4bit" | "4-bit" | "bits4" | "q4" => Ok(Self::Bits4),
            "8bit" | "8-bit" | "bits8" | "q8" => Ok(Self::Bits8),
            "16bit" | "16-bit" | "bits16" | "fp16" | "bf16" => Ok(Self::Bits16),
            "none" | "fp32" | "full" => Ok(Self::None),
            other => Err(StrataError::ValidationError(format!(
                "Unknown quantization type '{}'. Supported: 4bit, 8bit, 16bit, none",
                other
            ))),
        }
    }
}

/// Complete configuration parameters for Unsloth / Ollama LoRA fine-tuning.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrainingConfig {
    /// HuggingFace base model identifier (e.g. "unsloth/Llama-3.2-3B-Instruct")
    pub base_model: String,
    /// Training method (DPO, SFT, ORPO, KTO)
    pub method: TrainingMethod,
    /// Quantization for model loading
    pub quantization: QuantizationType,
    /// LoRA rank dimension
    pub lora_r: u32,
    /// LoRA alpha scaling parameter
    pub lora_alpha: u32,
    /// LoRA dropout probability (0.0 recommended for Unsloth)
    pub lora_dropout: f32,
    /// Target attention/MLP linear modules for LoRA injection
    pub target_modules: Vec<String>,
    /// Learning rate for AdamW optimizer
    pub learning_rate: f64,
    /// Per-device training batch size
    pub batch_size: usize,
    /// Gradient accumulation steps
    pub gradient_accumulation_steps: usize,
    /// Total training steps
    pub max_steps: usize,
    /// Maximum sequence length (context window for training)
    pub max_seq_length: usize,
    /// DPO / ORPO temperature / beta hyperparameter
    pub beta: f64,
    /// Random seed for reproducibility
    pub seed: u64,
    /// Output directory for LoRA adapters and artifacts
    pub output_dir: String,
    /// Explicit dataset path (if None, mined from Strata memory)
    pub dataset_path: Option<String>,
    /// Optional Ollama model name to register (e.g. "strata-custom-coder")
    pub ollama_model_name: Option<String>,
    /// Whether to export GGUF format for Ollama
    pub export_gguf: bool,
    /// GGUF quantization format (e.g. "q4_k_m", "q8_0", "f16")
    pub gguf_quantization: String,
}

impl Default for TrainingConfig {
    fn default() -> Self {
        Self {
            base_model: "unsloth/Llama-3.2-3B-Instruct".to_string(),
            method: TrainingMethod::Dpo,
            quantization: QuantizationType::Bits4,
            lora_r: 16,
            lora_alpha: 32,
            lora_dropout: 0.0,
            target_modules: vec![
                "q_proj".to_string(),
                "k_proj".to_string(),
                "v_proj".to_string(),
                "o_proj".to_string(),
                "gate_proj".to_string(),
                "up_proj".to_string(),
                "down_proj".to_string(),
            ],
            learning_rate: 5e-5,
            batch_size: 2,
            gradient_accumulation_steps: 4,
            max_steps: 60,
            max_seq_length: 2048,
            beta: 0.1,
            seed: 3407,
            output_dir: "outputs/lora_adapter".to_string(),
            dataset_path: None,
            ollama_model_name: Some("strata-custom-coder".to_string()),
            export_gguf: true,
            gguf_quantization: "q4_k_m".to_string(),
        }
    }
}

impl TrainingConfig {
    pub fn new(base_model: impl Into<String>) -> Self {
        Self {
            base_model: base_model.into(),
            ..Default::default()
        }
    }

    pub fn with_method(mut self, method: TrainingMethod) -> Self {
        self.method = method;
        self
    }

    pub fn with_quantization(mut self, quant: QuantizationType) -> Self {
        self.quantization = quant;
        self
    }

    pub fn with_lora(mut self, r: u32, alpha: u32, dropout: f32) -> Self {
        self.lora_r = r;
        self.lora_alpha = alpha;
        self.lora_dropout = dropout;
        self
    }

    pub fn with_learning_rate(mut self, lr: f64) -> Self {
        self.learning_rate = lr;
        self
    }

    pub fn with_batch_size(mut self, batch_size: usize, grad_accum: usize) -> Self {
        self.batch_size = batch_size;
        self.gradient_accumulation_steps = grad_accum;
        self
    }

    pub fn with_max_steps(mut self, steps: usize) -> Self {
        self.max_steps = steps;
        self
    }

    pub fn with_max_seq_length(mut self, len: usize) -> Self {
        self.max_seq_length = len;
        self
    }

    pub fn with_output_dir(mut self, dir: impl Into<String>) -> Self {
        self.output_dir = dir.into();
        self
    }

    pub fn with_dataset_path(mut self, path: impl Into<String>) -> Self {
        self.dataset_path = Some(path.into());
        self
    }

    pub fn with_ollama_model(mut self, model_name: impl Into<String>) -> Self {
        self.ollama_model_name = Some(model_name.into());
        self
    }

    pub fn validate(&self) -> Result<(), StrataError> {
        if self.base_model.trim().is_empty() {
            return Err(StrataError::ValidationError(
                "Base model cannot be empty".to_string(),
            ));
        }
        if self.lora_r == 0 {
            return Err(StrataError::ValidationError(
                "LoRA rank (lora_r) must be greater than 0".to_string(),
            ));
        }
        if self.lora_alpha == 0 {
            return Err(StrataError::ValidationError(
                "LoRA alpha must be greater than 0".to_string(),
            ));
        }
        if self.learning_rate <= 0.0 || self.learning_rate.is_nan() {
            return Err(StrataError::ValidationError(
                "Learning rate must be positive and non-zero".to_string(),
            ));
        }
        if self.batch_size == 0 {
            return Err(StrataError::ValidationError(
                "Batch size must be at least 1".to_string(),
            ));
        }
        if self.gradient_accumulation_steps == 0 {
            return Err(StrataError::ValidationError(
                "Gradient accumulation steps must be at least 1".to_string(),
            ));
        }
        if self.max_seq_length < 128 {
            return Err(StrataError::ValidationError(
                "Max sequence length must be at least 128 tokens".to_string(),
            ));
        }
        if self.target_modules.is_empty() {
            return Err(StrataError::ValidationError(
                "At least one LoRA target module must be specified".to_string(),
            ));
        }
        Ok(())
    }
}

/// Durable manifest metadata recorded upon training artifact generation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TrainingManifest {
    pub id: String,
    pub created_at: DateTime<Utc>,
    pub base_model: String,
    pub method: TrainingMethod,
    pub total_samples: usize,
    pub config: TrainingConfig,
    pub script_path: String,
    pub dataset_path: String,
    pub modelfile_path: Option<String>,
    pub run_script_path: Option<String>,
    pub ollama_model_name: Option<String>,
    pub status: String,
}

/// Outcome of training pipeline synthesis and artifact generation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TrainingResult {
    pub success: bool,
    pub manifest: TrainingManifest,
    pub script_path: String,
    pub dataset_path: String,
    pub modelfile_path: Option<String>,
    pub run_script_path: Option<String>,
    pub summary: String,
    pub preview_code: Option<String>,
}
