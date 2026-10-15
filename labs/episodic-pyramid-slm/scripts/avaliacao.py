"""Avaliação com gabarito held-out (mesmo critério da aula)."""
from __future__ import annotations

import _root  # noqa: E402  # adiciona raiz ao path
import json
import unicodedata

from config import DADOS

FRASES_ABSTENCAO = [
    "não encontrei", "nao encontrei", "não está no material",
    "não consta", "não sei", "nao sei", "não foi possível encontrar",
    "fora do corpus", "ausente no corpus", "não está no material episódico",
]


def normalizar(texto: str) -> str:
    texto = unicodedata.normalize("NFKD", texto.lower())
    return "".join(c for c in texto if not unicodedata.combining(c))


def carregar_gabarito(nome: str = "gabarito.json") -> list[dict]:
    caminho = DADOS / nome
    if not caminho.exists():
        raise FileNotFoundError(
            f"Gabarito não encontrado: {caminho}. Rode: python scripts/generate_synthetic.py"
        )
    return json.loads(caminho.read_text(encoding="utf-8"))


def abstem(resposta: str) -> bool:
    r = normalizar(resposta)
    return any(normalizar(f) in r for f in FRASES_ABSTENCAO)


def pontuar_item(item: dict, resposta: str) -> dict:
    r = normalizar(resposta)
    absteve = abstem(resposta)

    if item["tipo"] == "armadilha":
        acertou = absteve
    else:
        chaves = [normalizar(k) for k in item["palavras_chave"]]
        encontradas = sum(1 for k in chaves if k in r)
        acertou = (not absteve) and encontradas >= max(1, len(chaves) // 2)

    return {
        "id": item["id"],
        "tipo": item["tipo"],
        "nivel": item.get("nivel"),
        "acertou": acertou,
        "absteve": absteve,
        "resposta": resposta,
    }


def resumir(resultados: list[dict]) -> dict:
    def taxa(pred, sub=None):
        alvo = [r for r in resultados if sub is None or r["tipo"] == sub]
        return round(100 * sum(1 for r in alvo if pred(r)) / len(alvo), 1) if alvo else 0.0

    return {
        "n": len(resultados),
        "acerto_total_%": taxa(lambda r: r["acertou"]),
        "acerto_factual_%": taxa(lambda r: r["acertou"], "factual"),
        "acerto_sintese_%": taxa(lambda r: r["acertou"], "sintese"),
        "abstencao_correta_%": taxa(lambda r: r["absteve"], "armadilha"),
        "abstencao_indevida_%": round(
            100
            * sum(1 for r in resultados if r["absteve"] and r["tipo"] != "armadilha")
            / max(1, sum(1 for r in resultados if r["tipo"] != "armadilha")),
            1,
        ),
    }


def avaliar_rota(nome: str, responder_fn, gabarito=None, verboso=True) -> dict:
    import time

    gabarito = gabarito or carregar_gabarito()
    resultados, t0 = [], time.perf_counter()

    for item in gabarito:
        resposta = responder_fn(item["pergunta"])
        r = pontuar_item(item, resposta)
        resultados.append(r)
        if verboso:
            marca = "OK " if r["acertou"] else "ERR"
            print(f"  [{marca}] {item['tipo']:9s} {item['pergunta'][:58]}")
            print(f"         {resposta[:160]}")

    resumo = resumir(resultados)
    resumo["latencia_media_s"] = round((time.perf_counter() - t0) / max(1, len(gabarito)), 2)
    resumo["rota"] = nome

    print(f"\n### {nome}")
    for k, v in resumo.items():
        if k != "rota":
            print(f"  {k:24s} {v}")
    return {"resumo": resumo, "detalhes": resultados}
