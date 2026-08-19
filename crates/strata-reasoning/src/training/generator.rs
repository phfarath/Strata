use super::types::{QuantizationType, TrainingConfig, TrainingMethod};

/// Synthesize a production-ready Unsloth Python fine-tuning script.
pub fn generate_unsloth_training_script(config: &TrainingConfig, dataset_path: &str) -> String {
    let load_in_4bit = match config.quantization {
        QuantizationType::Bits4 => "True",
        _ => "False",
    };

    let target_modules_json = serde_json::to_string(&config.target_modules)
        .unwrap_or_else(|_| "[\"q_proj\", \"k_proj\", \"v_proj\", \"o_proj\", \"gate_proj\", \"up_proj\", \"down_proj\"]".to_string());

    let normalized_dataset = dataset_path.replace('\\', "/");
    let normalized_out = config.output_dir.replace('\\', "/");

    let trainer_setup = match config.method {
        TrainingMethod::Dpo => format!(
            r####"    # --- DPO Dataset Formatting ---
    raw_dataset = load_dataset("json", data_files="{dataset_path}")["train"]

    def format_dpo(sample):
        return {{
            "prompt": sample.get("prompt", ""),
            "chosen": sample.get("chosen", ""),
            "rejected": sample.get("rejected", ""),
        }}

    dataset = raw_dataset.map(format_dpo)

    # --- DPO Trainer Setup ---
    dpo_config = DPOConfig(
        output_dir = "{out_dir}",
        learning_rate = {lr},
        per_device_train_batch_size = {batch_size},
        gradient_accumulation_steps = {grad_accum},
        max_steps = {max_steps},
        max_length = {max_seq_len},
        max_prompt_length = {max_seq_len} // 2,
        beta = {beta},
        seed = {seed},
        fp16 = not is_bfloat16_supported(),
        bf16 = is_bfloat16_supported(),
        logging_steps = 1,
        optim = "adamw_8bit",
        weight_decay = 0.01,
        warmup_ratio = 0.1,
        lr_scheduler_type = "linear",
    )

    trainer = DPOTrainer(
        model = model,
        ref_model = None,
        args = dpo_config,
        train_dataset = dataset,
        tokenizer = tokenizer,
    )"####,
            dataset_path = normalized_dataset,
            out_dir = normalized_out,
            lr = config.learning_rate,
            batch_size = config.batch_size,
            grad_accum = config.gradient_accumulation_steps,
            max_steps = config.max_steps,
            max_seq_len = config.max_seq_length,
            beta = config.beta,
            seed = config.seed,
        ),
        TrainingMethod::Sft => format!(
            r####"    # --- SFT Dataset Formatting ---
    raw_dataset = load_dataset("json", data_files="{dataset_path}")["train"]

    def format_sft(sample):
        inst = sample.get("instruction", "")
        inp = sample.get("input", "")
        out = sample.get("output", "")
        if inp:
            text = "### Instruction:\n" + str(inst) + "\n\n### Input:\n" + str(inp) + "\n\n### Response:\n" + str(out)
        else:
            text = "### Instruction:\n" + str(inst) + "\n\n### Response:\n" + str(out)
        return {{"text": text}}

    dataset = raw_dataset.map(format_sft)

    # --- SFT Trainer Setup ---
    sft_config = SFTConfig(
        output_dir = "{out_dir}",
        dataset_text_field = "text",
        max_seq_length = {max_seq_len},
        dataset_num_proc = 2,
        packing = False,
        learning_rate = {lr},
        per_device_train_batch_size = {batch_size},
        gradient_accumulation_steps = {grad_accum},
        max_steps = {max_steps},
        seed = {seed},
        fp16 = not is_bfloat16_supported(),
        bf16 = is_bfloat16_supported(),
        logging_steps = 1,
        optim = "adamw_8bit",
        weight_decay = 0.01,
        warmup_ratio = 0.1,
        lr_scheduler_type = "linear",
    )

    trainer = SFTTrainer(
        model = model,
        tokenizer = tokenizer,
        train_dataset = dataset,
        args = sft_config,
    )"####,
            dataset_path = normalized_dataset,
            out_dir = normalized_out,
            lr = config.learning_rate,
            batch_size = config.batch_size,
            grad_accum = config.gradient_accumulation_steps,
            max_steps = config.max_steps,
            max_seq_len = config.max_seq_length,
            seed = config.seed,
        ),
        TrainingMethod::Orpo => format!(
            r####"    # --- ORPO Dataset Formatting ---
    raw_dataset = load_dataset("json", data_files="{dataset_path}")["train"]

    def format_orpo(sample):
        return {{
            "prompt": sample.get("prompt", ""),
            "chosen": sample.get("chosen", ""),
            "rejected": sample.get("rejected", ""),
        }}

    dataset = raw_dataset.map(format_orpo)

    # --- ORPO Trainer Setup ---
    orpo_config = ORPOConfig(
        output_dir = "{out_dir}",
        learning_rate = {lr},
        per_device_train_batch_size = {batch_size},
        gradient_accumulation_steps = {grad_accum},
        max_steps = {max_steps},
        max_length = {max_seq_len},
        max_prompt_length = {max_seq_len} // 2,
        beta = {beta},
        seed = {seed},
        fp16 = not is_bfloat16_supported(),
        bf16 = is_bfloat16_supported(),
        logging_steps = 1,
        optim = "adamw_8bit",
    )

    trainer = ORPOTrainer(
        model = model,
        args = orpo_config,
        train_dataset = dataset,
        tokenizer = tokenizer,
    )"####,
            dataset_path = normalized_dataset,
            out_dir = normalized_out,
            lr = config.learning_rate,
            batch_size = config.batch_size,
            grad_accum = config.gradient_accumulation_steps,
            max_steps = config.max_steps,
            max_seq_len = config.max_seq_length,
            beta = config.beta,
            seed = config.seed,
        ),
        TrainingMethod::Kto => format!(
            r####"    # --- KTO Dataset Formatting ---
    raw_dataset = load_dataset("json", data_files="{dataset_path}")["train"]

    def format_kto(sample):
        return {{
            "prompt": sample.get("prompt", ""),
            "completion": sample.get("completion", ""),
            "label": sample.get("label", True),
        }}

    dataset = raw_dataset.map(format_kto)

    # --- DPO/KTO Trainer Setup ---
    dpo_config = DPOConfig(
        output_dir = "{out_dir}",
        learning_rate = {lr},
        per_device_train_batch_size = {batch_size},
        gradient_accumulation_steps = {grad_accum},
        max_steps = {max_steps},
        max_length = {max_seq_len},
        beta = {beta},
        seed = {seed},
        fp16 = not is_bfloat16_supported(),
        bf16 = is_bfloat16_supported(),
        logging_steps = 1,
        optim = "adamw_8bit",
    )

    trainer = DPOTrainer(
        model = model,
        args = dpo_config,
        train_dataset = dataset,
        tokenizer = tokenizer,
    )"####,
            dataset_path = normalized_dataset,
            out_dir = normalized_out,
            lr = config.learning_rate,
            batch_size = config.batch_size,
            grad_accum = config.gradient_accumulation_steps,
            max_steps = config.max_steps,
            max_seq_len = config.max_seq_length,
            beta = config.beta,
            seed = config.seed,
        ),
    };

    let gguf_export_block = if config.export_gguf {
        format!(
            r####"
    # --- Export GGUF for Ollama Deployment ---
    print("\n📦 Exporting Fine-Tuned Model to GGUF format ({quant})...")
    gguf_dir = os.path.join("{out_dir}", "gguf")
    os.makedirs(gguf_dir, exist_ok=True)
    try:
        model.save_pretrained_gguf(gguf_dir, tokenizer, quantization_method="{quant}")
        print(f"✓ GGUF export complete: {{gguf_dir}}")
    except Exception as e:
        print(f"⚠️ GGUF export warning: {{e}} (LoRA adapter was saved successfully)")
"####,
            out_dir = normalized_out,
            quant = config.gguf_quantization,
        )
    } else {
        String::new()
    };

    format!(
        r####"#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
Strata Cognitive Runtime — Automated Local LoRA Fine-Tuning Script
Base Model: {base_model}
Method: {method} ({quant_type})
Generated by Strata Cognitive Pipeline
"""

import os
import sys
import torch
from datasets import load_dataset
from trl import DPOTrainer, DPOConfig, SFTTrainer, SFTConfig, ORPOTrainer, ORPOConfig
from unsloth import FastLanguageModel, is_bfloat16_supported

def main():
    print("=================================================================")
    print("🚀 STRATA ONE-CLICK LOCAL LORA FINE-TUNING VIA UNSLOTH")
    print("=================================================================")
    print(f"Base Model:    {base_model}")
    print(f"Method:        {method}")
    print(f"LoRA Rank:     {lora_r} (Alpha: {lora_alpha}, Dropout: {lora_dropout})")
    print(f"Max Context:   {max_seq_len} tokens")
    print(f"Learning Rate: {lr}")
    print(f"Max Steps:     {max_steps}")
    print("=================================================================\n")

    max_seq_length = {max_seq_len}
    load_in_4bit = {load_in_4bit}

    # 1. Load Base Model and Tokenizer
    print("⏳ Loading base model and tokenizer with Unsloth FastLanguageModel...")
    model, tokenizer = FastLanguageModel.from_pretrained(
        model_name = "{base_model}",
        max_seq_length = max_seq_length,
        dtype = None,
        load_in_4bit = load_in_4bit,
    )

    # 2. Inject LoRA PEFT Adapters
    print("🔧 Injecting LoRA parameter-efficient adapters...")
    model = FastLanguageModel.get_peft_model(
        model,
        r = {lora_r},
        target_modules = {target_modules},
        lora_alpha = {lora_alpha},
        lora_dropout = {lora_dropout},
        bias = "none",
        use_gradient_checkpointing = "unsloth",
        random_state = {seed},
        use_rslora = False,
        loftq_config = None,
    )

    # 3. Setup Dataset and Trainer
{trainer_setup}

    # 4. Train LoRA Adapters
    print("\n⚡ Starting LoRA training loop...")
    trainer_stats = trainer.train()
    print(f"✓ Training finished! Total runtime: {{trainer_stats.metrics.get('train_runtime', 0):.2f}}s")

    # 5. Save LoRA Adapters & Tokenizer
    out_dir = "{out_dir}"
    os.makedirs(out_dir, exist_ok=True)
    print(f"💾 Saving LoRA adapter weights to '{{out_dir}}'...")
    model.save_pretrained(out_dir)
    tokenizer.save_pretrained(out_dir)
    print("✓ LoRA adapter saved successfully.")
{gguf_export_block}
    print("\n🎉 Strata LoRA fine-tuning pipeline completed successfully!")

if __name__ == "__main__":
    main()
"####,
        base_model = config.base_model,
        method = config.method,
        quant_type = config.quantization,
        max_seq_len = config.max_seq_length,
        lr = config.learning_rate,
        max_steps = config.max_steps,
        load_in_4bit = load_in_4bit,
        lora_r = config.lora_r,
        lora_alpha = config.lora_alpha,
        lora_dropout = config.lora_dropout,
        target_modules = target_modules_json,
        seed = config.seed,
        trainer_setup = trainer_setup,
        out_dir = normalized_out,
        gguf_export_block = gguf_export_block,
    )
}

/// Synthesize an Ollama `Modelfile` to register and run the fine-tuned model locally.
pub fn generate_ollama_modelfile(config: &TrainingConfig, adapter_or_gguf_path: &str) -> String {
    let normalized_path = adapter_or_gguf_path.replace('\\', "/");
    let is_gguf = normalized_path.ends_with(".gguf") || normalized_path.contains("/gguf");

    let from_line = if is_gguf {
        format!("FROM {}", normalized_path)
    } else {
        format!("FROM {}\nADAPTER {}", config.base_model, normalized_path)
    };

    format!(
        r####"# Strata Cognitive Runtime — Ollama Modelfile
# Auto-generated by Strata One-Click LoRA Pipeline
# Base Model: {base_model}
# Method: {method}

{from_line}

# Runtime Sampling Parameters
PARAMETER temperature 0.2
PARAMETER top_p 0.95
PARAMETER top_k 40
PARAMETER stop "<|im_end|>"
PARAMETER stop "<|endoftext|>"
PARAMETER stop "</s>"

# System Prompt Infused with Strata Continuous Learning Protocol
SYSTEM """You are a specialized Strata-aligned cognitive coding assistant, fine-tuned on verified architectural patterns, negative failure mitigations, and contextual episodic memories. Prioritize atomic, verified code, avoid known anti-patterns, and adhere strictly to the project's Radical Simplicity Principle."""
"####,
        base_model = config.base_model,
        method = config.method,
        from_line = from_line,
    )
}

/// Synthesize a cross-platform execution script (`run_training.sh`) for one-click fine-tuning.
pub fn generate_run_script(config: &TrainingConfig, script_path: &str, output_dir: &str) -> String {
    let normalized_script = script_path.replace('\\', "/");
    let normalized_out = output_dir.replace('\\', "/");

    let ollama_deploy = if let Some(model_name) = &config.ollama_model_name {
        format!(
            r####"
# Deploy model to local Ollama instance if available
if command -v ollama &> /dev/null; then
    echo "🦙 Creating Ollama model '{model_name}' from Modelfile..."
    if [ -f "{out_dir}/Modelfile" ]; then
        ollama create "{model_name}" -f "{out_dir}/Modelfile"
        echo "✓ Model '{model_name}' registered in Ollama. Test with: ollama run {model_name}"
    fi
else
    echo "ℹ️ Ollama CLI not detected on PATH. Install from https://ollama.com to deploy locally."
fi
"####,
            model_name = model_name,
            out_dir = normalized_out,
        )
    } else {
        String::new()
    };

    format!(
        r####"#!/usr/bin/env bash
set -e

echo "=========================================================="
echo "⚡ STRATA ONE-CLICK LORA TRAINING RUNNER"
echo "=========================================================="

# Check Python environment
if ! command -v python3 &> /dev/null && ! command -v python &> /dev/null; then
    echo "❌ Error: Python is required to run Unsloth fine-tuning."
    exit 1
fi

PYTHON_CMD="python3"
if ! command -v python3 &> /dev/null; then
    PYTHON_CMD="python"
fi

echo "📦 Executing Unsloth training script: {script_path}"
$PYTHON_CMD "{script_path}"
{ollama_deploy}
echo "=========================================================="
echo "🎉 LoRA fine-tuning process finished successfully!"
echo "=========================================================="
"####,
        script_path = normalized_script,
        ollama_deploy = ollama_deploy,
    )
}
