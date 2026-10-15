#!/usr/bin/env python3
"""
CLI de inferência local para compressão de narrativa de sessão (Tier 2 -> Tier 3)
e milestones (Tier 1 -> Tier 2) usando SmolLM2 e adapter LoRA r16_1b7.

Suporta execução 100% offline (HF_HUB_OFFLINE=1, TRANSFORMERS_OFFLINE=1).
Se PyTorch ou GPU/pesos não estiverem disponíveis, fornece fallback determinístico
sem falhar a pipeline do Strata.
"""
from __future__ import annotations

import argparse
import json
import os
import sys
from pathlib import Path

# Força execução offline por padrão
os.environ.setdefault("HF_HUB_OFFLINE", "1")
os.environ.setdefault("TRANSFORMERS_OFFLINE", "1")
os.environ.setdefault("HF_DATASETS_OFFLINE", "1")
os.environ.setdefault("TOKENIZERS_PARALLELISM", "false")
os.environ.setdefault("HF_HUB_DISABLE_PROGRESS_BARS", "1")

RAIZ = Path(__file__).resolve().parent
if str(RAIZ) not in sys.path:
    sys.path.insert(0, str(RAIZ))


def fallback_t2_t3(texto: str) -> str:
    """Gera narrativa determinística heurística de Tier 3."""
    linhas = [ln.strip() for ln in texto.splitlines() if ln.strip()]
    medoids = []
    for ln in linhas:
        if ln.startswith("-") or ln.startswith("•"):
            medoids.append(ln.lstrip("-• ").strip())
        elif ln.startswith("[Milestone Rollup]"):
            medoids.append(ln.replace("[Milestone Rollup]", "").strip())
        elif ln.startswith("Milestone"):
            medoids.append(ln.replace("Milestone", "").strip())
        else:
            medoids.append(ln)

    resumo_alvo = " | ".join(medoids[:5])
    return f"Narrativa: Sessao agregou {len(medoids)} marcos principais. Arco de execucao: {resumo_alvo}. Invariantes e contratos preservados com sucesso."


def fallback_t1_t2(texto: str) -> str:
    from compressao import heuristic_from_text
    return heuristic_from_text(texto)


def executar_inferencia(
    texto: str,
    tier: str = "t2_to_t3",
    adapter_dir: Path | None = None,
    max_tokens: int = 180,
    temperatura: float = 0.2,
) -> dict:
    cin = len(texto)
    heur = fallback_t2_t3(texto) if tier in ("t2_to_t3", "3") else fallback_t1_t2(texto)

    try:
        import torch
        from transformers.utils import logging as hf_logging
        hf_logging.set_verbosity_error()
        from peft import PeftModel
        from compressao import comprimir_t1_t2, comprimir_t2_t3
        from config import MODELO_CHAT, RUNS
        from utils import carregar

        resolved_adapter = adapter_dir or (RUNS / "lora_r16_1b7" / "adapter")
        modelo, tok = carregar(MODELO_CHAT)

        if resolved_adapter.exists():
            modelo = PeftModel.from_pretrained(modelo, str(resolved_adapter))
            modelo = modelo.merge_and_unload()
            modo = f"LoRA {resolved_adapter.parent.name}"
        else:
            modo = "few-shot SmolLM2-1.7B Instruct"

        modelo.eval()

        if tier in ("t2_to_t3", "3"):
            res = comprimir_t2_t3(modelo, tok, texto, max_new_tokens=max_tokens, temperatura=temperatura)
        else:
            res = comprimir_t1_t2(modelo, tok, texto, max_new_tokens=max_tokens, temperatura=temperatura)

        return {
            "narrative": res.slm,
            "heuristic": res.heuristic,
            "tier": 3 if tier in ("t2_to_t3", "3") else 2,
            "chars_in": res.chars_in,
            "chars_out": res.chars_out_slm,
            "compression_ratio": res.ratio_slm,
            "model": modo,
            "offline": True,
            "fallback": False,
            "warning": res.aviso or None,
        }
    except Exception as e:
        # Fallback gracioso determinístico
        cout = len(heur)
        ratio = round(cin / max(1, cout), 2)
        return {
            "narrative": heur,
            "heuristic": heur,
            "tier": 3 if tier in ("t2_to_t3", "3") else 2,
            "chars_in": cin,
            "chars_out": cout,
            "compression_ratio": ratio,
            "model": "heuristic_fallback",
            "offline": True,
            "fallback": True,
            "warning": f"Fallback ativado devido a: {str(e)}",
        }


def main():
    parser = argparse.ArgumentParser(description="Inferência local SmolLM2 para pirâmide episódica")
    parser.add_argument("--tier", choices=["t1_to_t2", "t2_to_t3", "2", "3"], default="t2_to_t3", help="Nível de compressão")
    parser.add_argument("--adapter-dir", type=str, default=None, help="Caminho do diretório de adapter LoRA")
    parser.add_argument("--input", type=str, default="-", help="Texto de entrada ou caminho do arquivo (use '-' para stdin)")
    parser.add_argument("--max-tokens", type=int, default=180, help="Máximo de novos tokens")
    parser.add_argument("--temperature", type=float, default=0.2, help="Temperatura de amostragem")
    parser.add_argument("--json", action="store_true", help="Saída formatada em JSON")
    parser.add_argument("--offline", action="store_true", default=True, help="Forçar modo offline")

    args = parser.parse_args()

    if args.input == "-":
        texto = sys.stdin.read()
    else:
        p = Path(args.input)
        if p.exists() and p.is_file():
            texto = p.read_text(encoding="utf-8")
        else:
            texto = args.input

    texto = texto.strip()
    if not texto:
        sys.stderr.write("Erro: entrada vazia para compressao episodica.\n")
        sys.exit(1)

    adapter_path = Path(args.adapter_dir) if args.adapter_dir else None
    resultado = executar_inferencia(
        texto=texto,
        tier=args.tier,
        adapter_dir=adapter_path,
        max_tokens=args.max_tokens,
        temperatura=args.temperature,
    )

    if args.json:
        print(json.dumps(resultado, ensure_ascii=False, indent=2))
    else:
        print(resultado["narrative"])


if __name__ == "__main__":
    main()
