"""Avalia um adapter LoRA (ou o modelo base) no gabarito."""
from __future__ import annotations

import _root  # noqa: E402  # adiciona raiz ao path
import argparse
from pathlib import Path

from peft import PeftModel

from avaliacao import avaliar_rota
from config import MODELO_CHAT, RUNS, SISTEMA_T1_T2, SISTEMA_T2_T3, resumo_ambiente
from utils import carregar, chat, fixar_seed, liberar, medir, salvar_resultado


def parse_args():
    p = argparse.ArgumentParser()
    p.add_argument("--adapter", type=str, default=None, help="Pasta do adapter LoRA")
    p.add_argument("--tag", type=str, default="default")
    p.add_argument("--nome", type=str, default=None, help="Nome do resultado")
    return p.parse_args()


def main() -> None:
    args = parse_args()
    fixar_seed()
    print(resumo_ambiente())

    adapter = Path(args.adapter) if args.adapter else RUNS / f"lora_{args.tag}" / "adapter"
    nome = args.nome or f"lora_{args.tag}"

    modelo, tok = carregar(MODELO_CHAT)
    if adapter.exists():
        print(f"Carregando adapter: {adapter}")
        modelo = PeftModel.from_pretrained(modelo, str(adapter))
        modelo = modelo.merge_and_unload()
    else:
        print(f"[aviso] adapter não encontrado em {adapter}; avaliando base")

    def responder(pergunta: str) -> str:
        sistema = SISTEMA_T2_T3 if "Tier 3" in pergunta or "narrativa" in pergunta.lower() else SISTEMA_T1_T2
        return chat(modelo, tok, pergunta, sistema=sistema, max_new_tokens=220)

    with medir(f"eval {nome}"):
        resultado = avaliar_rota(nome, responder)

    salvar_resultado(nome, resultado)
    liberar(modelo, tok)


if __name__ == "__main__":
    main()
