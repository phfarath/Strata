"""
Bake-off: heurística Strata vs baseline vs LoRA(s).

Lê runs/resultados/*.json e monta tabela + gráfico.
Também imprime comparação qualitativa heuristic vs gold.
"""
from __future__ import annotations

import _root  # noqa: E402  # adiciona raiz ao path
import json

import matplotlib

matplotlib.use("Agg")
import matplotlib.pyplot as plt

from config import DADOS, RUNS
from utils import carregar_resultado

ROTAS = [
    ("baseline", "Baseline\n(sem FT)"),
    ("lora_default", "LoRA\ndefault"),
    ("lora_r8_lr1e4", "LoRA\nr8 lr1e4"),
    ("lora_r16_t23", "LoRA\n360M"),
    ("lora_r16_1b7", "LoRA\n1.7B"),
]

METRICAS = [
    ("acerto_total_%", "acerto geral"),
    ("acerto_factual_%", "acerto factual"),
    ("acerto_sintese_%", "acerto síntese"),
    ("abstencao_correta_%", "abstenção correta"),
    ("abstencao_indevida_%", "abstenção indevida"),
    ("latencia_media_s", "latência (s)"),
]


def main() -> None:
    dados = {}
    for chave, rotulo in ROTAS:
        r = carregar_resultado(chave)
        if r is None:
            print(f"[falta] {chave}")
            continue
        dados[rotulo] = r.get("resumo") or r

    # Heurística: pontuação proxy (não passa pelo modelo)
    heur_path = DADOS / "heuristic_baselines.json"
    if heur_path.exists():
        heur = json.loads(heur_path.read_text(encoding="utf-8"))
        print("\n=== Heurística Strata vs Gold (qualitativo) ===")
        for task_id, row in heur.items():
            print(f"\n[{task_id}] n={row['n_episodes']}")
            print(f"  heurística: {row['heuristic'][:120]}...")
            print(f"  gold SLM  : {row['gold'][:120]}...")

    if not dados:
        raise SystemExit("Nenhum resultado em runs/resultados/. Rode baseline/evaluate primeiro.")

    largura = max(len(r.replace("\n", " ")) for r in dados) + 2
    print(f"\n{'métrica':22s}" + "".join(f"{r.replace(chr(10), ' '):>16s}" for r in dados))
    print("-" * (22 + 16 * len(dados)))
    for chave, rotulo in METRICAS:
        linha = f"{rotulo:22s}"
        for r in dados:
            linha += f"{dados[r].get(chave, float('nan')):>16.1f}"
        print(linha)

    fig, ax = plt.subplots(figsize=(9, 4.5))
    rotulos = list(dados)
    x = range(len(rotulos))
    for i, (chave, nome) in enumerate([("acerto_total_%", "acerto"), ("abstencao_correta_%", "abst. OK")]):
        vals = [dados[r].get(chave, 0) for r in rotulos]
        ax.bar([xi + i * 0.35 for xi in x], vals, width=0.35, label=nome)
    ax.set_xticks([xi + 0.17 for xi in x])
    ax.set_xticklabels([r.replace("\n", " ") for r in rotulos], fontsize=9)
    ax.set_ylabel("%")
    ax.set_title("Bake-off: compressão episódica Pyramid SLM")
    ax.legend()
    ax.set_ylim(0, 100)
    fig.tight_layout()
    out = RUNS / "bakeoff.png"
    fig.savefig(out, dpi=140)
    print(f"\nGráfico: {out}")


if __name__ == "__main__":
    main()
