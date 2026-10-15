"""Funções compartilhadas: carga, geração, VRAM, resultados."""
from __future__ import annotations

import json
import math
import random
import time
from contextlib import contextmanager
from pathlib import Path

import numpy as np
import torch
from transformers import AutoModelForCausalLM, AutoTokenizer

from config import ATTN, DEVICE, DTYPE, RUNS, SEED


def fixar_seed(seed: int = SEED) -> None:
    random.seed(seed)
    np.random.seed(seed)
    torch.manual_seed(seed)
    torch.cuda.manual_seed_all(seed)


def carregar_tokenizer(nome: str):
    tok = AutoTokenizer.from_pretrained(nome)
    if tok.pad_token is None:
        tok.pad_token = tok.eos_token
    return tok


def carregar_modelo(nome: str, **kwargs):
    comuns = dict(attn_implementation=ATTN, **kwargs)
    try:
        modelo = AutoModelForCausalLM.from_pretrained(nome, dtype=DTYPE, **comuns)
    except TypeError:
        modelo = AutoModelForCausalLM.from_pretrained(nome, torch_dtype=DTYPE, **comuns)
    return modelo.to(DEVICE)


def carregar(nome: str):
    tok = carregar_tokenizer(nome)
    modelo = carregar_modelo(nome)
    modelo.config.pad_token_id = tok.pad_token_id
    return modelo, tok


@torch.no_grad()
def chat(modelo, tok, pergunta: str, sistema: str | None = None,
         max_new_tokens: int = 256, temperatura: float = 0.0,
         repetition_penalty: float = 1.15) -> str:
    msgs = []
    if sistema:
        msgs.append({"role": "system", "content": sistema})
    msgs.append({"role": "user", "content": pergunta})

    texto = tok.apply_chat_template(msgs, tokenize=False, add_generation_prompt=True)
    entradas = tok(texto, return_tensors="pt").to(modelo.device)

    kw = dict(
        max_new_tokens=max_new_tokens,
        pad_token_id=tok.pad_token_id,
        eos_token_id=tok.eos_token_id,
        repetition_penalty=repetition_penalty,
    )
    if temperatura > 0:
        kw.update(do_sample=True, temperature=temperatura, top_p=0.9)
    else:
        kw.update(do_sample=False)

    saida = modelo.generate(**entradas, **kw)
    novos = saida[0][entradas["input_ids"].shape[1]:]
    texto_out = tok.decode(novos, skip_special_tokens=True).strip()
    # SmolLM2 às vezes abre com aspas e trava em loop """..."""; limpa e retenta leve.
    if _degenerado(texto_out) and temperatura == 0.0:
        kw.update(do_sample=True, temperature=0.4, top_p=0.9)
        saida = modelo.generate(**entradas, **kw)
        novos = saida[0][entradas["input_ids"].shape[1]:]
        texto_out = tok.decode(novos, skip_special_tokens=True).strip()
    return _limpar_aspas(texto_out)


def _degenerado(texto: str) -> bool:
    if not texto or len(texto) < 8:
        return True
    so_aspas = sum(1 for c in texto if c in "\"'`") / max(1, len(texto))
    return so_aspas > 0.4


def _limpar_aspas(texto: str) -> str:
    t = texto.strip().lstrip("\"'`").rstrip("\"'`").strip()
    # remove linhas que são só aspas
    linhas = [ln for ln in t.splitlines() if ln.strip().strip("\"'`")]
    return "\n".join(linhas).strip()


@torch.no_grad()
def perplexidade(modelo, tok, textos: list[str]) -> float:
    modelo.eval()
    nll_total, tokens_total = 0.0, 0
    for texto in textos:
        enc = tok(texto, return_tensors="pt", truncation=True, max_length=1024)
        enc = {k: v.to(modelo.device) for k, v in enc.items()}
        n = enc["input_ids"].numel() - 1
        if n <= 0:
            continue
        saida = modelo(**enc, labels=enc["input_ids"])
        nll_total += saida.loss.item() * n
        tokens_total += n
    return math.exp(nll_total / tokens_total) if tokens_total else float("inf")


def reset_vram() -> None:
    if DEVICE == "cuda":
        torch.cuda.empty_cache()
        torch.cuda.reset_peak_memory_stats()


def liberar(*objetos) -> None:
    import gc

    for o in objetos:
        del o
    gc.collect()
    if DEVICE == "cuda":
        torch.cuda.empty_cache()
        torch.cuda.ipc_collect()


def vram_pico_gb() -> float:
    return torch.cuda.max_memory_allocated() / 1024**3 if DEVICE == "cuda" else 0.0


@contextmanager
def medir(rotulo: str):
    reset_vram()
    t0 = time.perf_counter()
    yield
    dt = time.perf_counter() - t0
    print(f"[{rotulo}] tempo={dt:.1f}s  VRAM_pico={vram_pico_gb():.2f} GB")


def salvar_resultado(nome: str, dados: dict) -> Path:
    destino = RUNS / "resultados"
    destino.mkdir(parents=True, exist_ok=True)
    caminho = destino / f"{nome}.json"
    caminho.write_text(json.dumps(dados, ensure_ascii=False, indent=2), encoding="utf-8")
    print(f"  -> salvo em {caminho}")
    return caminho


def carregar_resultado(nome: str) -> dict | None:
    caminho = RUNS / "resultados" / f"{nome}.json"
    if not caminho.exists():
        return None
    return json.loads(caminho.read_text(encoding="utf-8"))


def contar_parametros(modelo) -> tuple[int, int]:
    total = sum(p.numel() for p in modelo.parameters())
    treinaveis = sum(p.numel() for p in modelo.parameters() if p.requires_grad)
    return total, treinaveis


def escrever_jsonl(caminho: Path, rows: list[dict]) -> None:
    caminho.parent.mkdir(parents=True, exist_ok=True)
    caminho.write_text(
        "\n".join(json.dumps(r, ensure_ascii=False) for r in rows) + ("\n" if rows else ""),
        encoding="utf-8",
    )


def ler_jsonl(caminho: Path) -> list[dict]:
    if not caminho.exists():
        return []
    return [
        json.loads(linha)
        for linha in caminho.read_text(encoding="utf-8").splitlines()
        if linha.strip()
    ]
