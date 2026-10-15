"""
SFT LoRA: ensina SmolLM2 a comprimir episódios Tier1->Tier2 (+ opcional Tier2->Tier3).

Usa transformers.Trainer + PEFT (compatível com Windows; TRL 1.15 exige Triton/Linux).

Uso:
  python train_lora.py
  python train_lora.py --r 8 --lr 1e-4 --epochs 1 --tag r8_lr1e4
  python train_lora.py --include-t23
"""
from __future__ import annotations

import argparse
import json

import torch
from datasets import Dataset
from peft import LoraConfig, TaskType, get_peft_model
from transformers import (
    AutoModelForCausalLM,
    AutoTokenizer,
    DataCollatorForLanguageModeling,
    Trainer,
    TrainingArguments,
)

from config import (
    ATTN,
    BATCH_SIZE,
    DADOS,
    DEVICE,
    DTYPE,
    EPOCHS,
    GRAD_ACCUM,
    LORA_ALPHA,
    LORA_DROPOUT,
    LORA_R,
    LR,
    MAX_SEQ_LEN,
    MODELO_CHAT,
    RUNS,
    SEED,
    USA_BF16,
    resumo_ambiente,
)
from utils import fixar_seed, liberar, medir, salvar_resultado, ler_jsonl


def parse_args():
    p = argparse.ArgumentParser()
    p.add_argument("--r", type=int, default=LORA_R)
    p.add_argument("--alpha", type=int, default=LORA_ALPHA)
    p.add_argument("--lr", type=float, default=LR)
    p.add_argument("--epochs", type=int, default=EPOCHS)
    p.add_argument("--max-seq", type=int, default=MAX_SEQ_LEN)
    p.add_argument("--include-t23", action="store_true")
    p.add_argument("--tag", type=str, default="default")
    p.add_argument("--modelo", type=str, default=None, help="Override do modelo base")
    return p.parse_args()


def montar_texto(tok, row: dict) -> str:
    msgs = [
        {"role": "system", "content": row["sistema"]},
        {"role": "user", "content": row["prompt"]},
        {"role": "assistant", "content": row["completion"]},
    ]
    return tok.apply_chat_template(msgs, tokenize=False)


def main() -> None:
    args = parse_args()
    fixar_seed()
    print(resumo_ambiente())
    assert DEVICE == "cuda", "Treino deve rodar na GPU"

    pares = ler_jsonl(DADOS / "sft_t1_to_t2.jsonl")
    if args.include_t23:
        pares += ler_jsonl(DADOS / "sft_t2_to_t3.jsonl")
    assert pares, "Sem dados SFT. Rode: python generate_synthetic.py"
    modelo_nome = args.modelo or MODELO_CHAT
    print(f"Pares de treino: {len(pares)}")
    print(f"Modelo base: {modelo_nome}")

    tok = AutoTokenizer.from_pretrained(modelo_nome)
    if tok.pad_token is None:
        tok.pad_token = tok.eos_token

    try:
        modelo = AutoModelForCausalLM.from_pretrained(
            modelo_nome, dtype=DTYPE, attn_implementation=ATTN
        )
    except TypeError:
        modelo = AutoModelForCausalLM.from_pretrained(
            modelo_nome, torch_dtype=DTYPE, attn_implementation=ATTN
        )
    modelo = modelo.to(DEVICE)
    modelo.config.pad_token_id = tok.pad_token_id
    modelo.config.use_cache = False

    lora = LoraConfig(
        r=args.r,
        lora_alpha=args.alpha,
        lora_dropout=LORA_DROPOUT,
        bias="none",
        task_type=TaskType.CAUSAL_LM,
        target_modules=["q_proj", "k_proj", "v_proj", "o_proj", "gate_proj", "up_proj", "down_proj"],
    )
    modelo = get_peft_model(modelo, lora)
    modelo.print_trainable_parameters()
    modelo.enable_input_require_grads()

    textos = [montar_texto(tok, p) for p in pares]

    def tokenize(batch):
        return tok(
            batch["text"],
            truncation=True,
            max_length=args.max_seq,
            padding=False,
        )

    ds = Dataset.from_dict({"text": textos}).map(tokenize, batched=True, remove_columns=["text"])
    if len(ds) >= 10:
        split = ds.train_test_split(test_size=0.15, seed=SEED)
        train_ds, eval_ds = split["train"], split["test"]
    else:
        train_ds, eval_ds = ds, None

    out_dir = RUNS / f"lora_{args.tag}"
    out_dir.mkdir(parents=True, exist_ok=True)

    targs = dict(
        output_dir=str(out_dir),
        num_train_epochs=args.epochs,
        per_device_train_batch_size=BATCH_SIZE,
        gradient_accumulation_steps=GRAD_ACCUM,
        learning_rate=args.lr,
        warmup_steps=5,
        lr_scheduler_type="cosine",
        bf16=USA_BF16,
        fp16=not USA_BF16,
        gradient_checkpointing=True,
        logging_steps=2,
        save_strategy="epoch",
        seed=SEED,
        report_to=[],
        dataloader_num_workers=0,
        remove_unused_columns=False,
    )
    if eval_ds is not None:
        targs["eval_strategy"] = "epoch"
        targs["per_device_eval_batch_size"] = 1

    training_args = TrainingArguments(**targs)
    collator = DataCollatorForLanguageModeling(tok, mlm=False)

    trainer = Trainer(
        model=modelo,
        args=training_args,
        train_dataset=train_ds,
        eval_dataset=eval_ds,
        data_collator=collator,
        processing_class=tok,
    )

    with medir(f"treino LoRA tag={args.tag}"):
        train_out = trainer.train()

    adapter_dir = out_dir / "adapter"
    trainer.model.save_pretrained(adapter_dir)
    tok.save_pretrained(adapter_dir)

    meta = {
        "tag": args.tag,
        "modelo": modelo_nome,
        "r": args.r,
        "alpha": args.alpha,
        "lr": args.lr,
        "epochs": args.epochs,
        "max_seq": args.max_seq,
        "include_t23": args.include_t23,
        "n_pares": len(pares),
        "train_loss": float(getattr(train_out, "training_loss", 0.0) or 0.0),
        "adapter": str(adapter_dir),
        "backend": "transformers.Trainer+PEFT",
    }
    (out_dir / "meta.json").write_text(json.dumps(meta, indent=2), encoding="utf-8")
    salvar_resultado(f"train_{args.tag}", meta)
    print(f"Adapter salvo em {adapter_dir}")

    liberar(trainer, modelo, tok)


if __name__ == "__main__":
    main()
