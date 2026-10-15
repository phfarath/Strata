"""
Compressão episódica: heurística Strata + SLM com few-shot.

O 360M sozinho alucina código com facilidade; few-shot + formato rígido
melhora a demo. A heurística sempre roda (determinística, como no Rust).
"""
from __future__ import annotations

import re
from dataclasses import dataclass

from config import SISTEMA_T1_T2, SISTEMA_T2_T3
from utils import chat

# Exemplos few-shot (gold do corpus) — ensinam o FORMATO na hora da inferência
FEWSHOT_T1_T2 = """### Exemplo
Entrada (episódios Tier 1 resumidos):
- Generate RSA keypair; Implement Claims RS256; Expose JWKS; Remove HS256; Redis revocation
Saida:
Milestone Auth JWT: migracao HS256->RS256. Keypair RSA, Claims/token, JWKS, removeu HS256, Redis para revogacao, middleware Bearer. Arquivos: src/auth/*.rs. Outcomes: RS256 operacional; HS256 deprecado.

### Agora comprima de verdade
Entrada:
{entrada}

Saida (3 a 6 frases em portugues, SEM codigo, SEM aspas, SEM novos episodios):
Milestone"""

FEWSHOT_T2_T3 = """### Exemplo
Entrada (milestones Tier 2):
- Auth JWT RS256+JWKS
- SQLite pool com DeadlockTimeout sob carga
- CDC outbox Kafka+DLQ
Saida:
Narrativa: sessao estabilizou auth assimetrica, endureceu o pool SQLite apos esgotamento, e entregou outbox confiavel. Decisoes: RS256, single-writer, SKIP LOCKED. Riscos: clientes HS256, lag do outbox.

### Agora comprima
Entrada:
{entrada}

Saida (4 a 8 frases, SEM codigo):
Narrativa"""


@dataclass
class ResultadoCompressao:
    heuristic: str
    slm: str
    chars_in: int
    chars_out_slm: int
    chars_out_heur: int
    ratio_slm: float
    ratio_heur: float
    aviso: str = ""


def _parece_codigo(texto: str) -> bool:
    t = texto.lower()
    marcas = ["fn ", "struct ", "impl ", "pub fn", "```", "openssl::", "use std::", "let mut ", "{"]
    hits = sum(1 for m in marcas if m in t)
    return hits >= 2 or texto.count("{") >= 2


def heuristic_from_text(texto: str) -> str:
    """Aproxima o template Strata a partir do texto colado."""
    summaries = re.findall(r"### Epis[oó]dio:\s*(.+)", texto, flags=re.I)
    outcomes = re.findall(r"outcomes?\s*=\s*\[([^\]]*)\]", texto, flags=re.I)
    files = re.findall(r"files?\s*=\s*\[([^\]]*)\]", texto, flags=re.I)
    n_out = 0
    for o in outcomes:
        n_out += len([x for x in o.split(",") if x.strip().strip("'\"")])
    n_files = 0
    seen = set()
    for fblock in files:
        for f in re.findall(r"['\"]([^'\"]+)['\"]", fblock):
            if f not in seen:
                seen.add(f)
                n_files += 1
    medoid = summaries[0].strip() if summaries else "cluster"
    # pega o de maior salience se houver
    blocks = re.split(r"### Epis[oó]dio:", texto, flags=re.I)
    best_sal, best_sum = -1.0, medoid
    for b in blocks[1:]:
        m_sum = b.split("\n", 1)[0].strip()
        m_sal = re.search(r"salience\s*=\s*([0-9.]+)", b)
        sal = float(m_sal.group(1)) if m_sal else 0.0
        if sal > best_sal:
            best_sal, best_sum = sal, m_sum or medoid
    return (
        f"[Milestone Rollup] {best_sum} | "
        f"Invariants: {n_out} outcomes, {n_files} files touched"
    )


def comprimir_t1_t2(modelo, tok, texto: str, max_new_tokens: int = 160, temperatura: float = 0.2) -> ResultadoCompressao:
    heur = heuristic_from_text(texto)
    prompt = FEWSHOT_T1_T2.format(entrada=texto.strip()[:3500])
    slm = chat(
        modelo,
        tok,
        prompt,
        sistema=SISTEMA_T1_T2,
        max_new_tokens=max_new_tokens,
        temperatura=temperatura,
        repetition_penalty=1.3,
    )
    # Garante prefixo legível
    if slm and not slm.lower().startswith("milestone"):
        slm = "Milestone " + slm.lstrip(": ").strip()

    aviso = ""
    if _parece_codigo(slm):
        aviso = "SLM gerou codigo (alucinacao). Tentando de novo com temperatura menor..."
        slm2 = chat(
            modelo,
            tok,
            prompt + "\nLembre: PROIBIDO codigo. So prosa em portugues.\nMilestone",
            sistema=SISTEMA_T1_T2,
            max_new_tokens=max_new_tokens,
            temperatura=0.1,
            repetition_penalty=1.35,
        )
        if slm2 and not slm2.lower().startswith("milestone"):
            slm2 = "Milestone " + slm2.lstrip(": ").strip()
        if not _parece_codigo(slm2):
            slm = slm2
            aviso = "Recuperado apos retry (1a tentativa veio codigo)."
        else:
            slm = (
                "Milestone (fallback): cluster Auth/DB/CDC detectado no prompt. "
                "O SLM alucinou codigo; use a heuristica ao lado como referencia "
                "deterministica do Strata ate retreinar com mais pares."
            )
            aviso = "SLM falhou (codigo). Mostrando fallback textual + heuristica."

    cin = len(texto)
    return ResultadoCompressao(
        heuristic=heur,
        slm=slm,
        chars_in=cin,
        chars_out_slm=len(slm),
        chars_out_heur=len(heur),
        ratio_slm=round(cin / max(1, len(slm)), 2),
        ratio_heur=round(cin / max(1, len(heur)), 2),
        aviso=aviso,
    )


def comprimir_t2_t3(modelo, tok, texto: str, max_new_tokens: int = 180, temperatura: float = 0.2) -> ResultadoCompressao:
    heur = "[Session Narrative] " + " | ".join(
        ln.strip()[:80] for ln in texto.splitlines() if ln.strip()
    )[:200]
    prompt = FEWSHOT_T2_T3.format(entrada=texto.strip()[:3500])
    slm = chat(
        modelo,
        tok,
        prompt,
        sistema=SISTEMA_T2_T3,
        max_new_tokens=max_new_tokens,
        temperatura=temperatura,
        repetition_penalty=1.3,
    )
    if slm and not slm.lower().startswith("narrativa"):
        slm = "Narrativa: " + slm.lstrip(": ").strip()
    cin = len(texto)
    return ResultadoCompressao(
        heuristic=heur,
        slm=slm,
        chars_in=cin,
        chars_out_slm=len(slm),
        chars_out_heur=len(heur),
        ratio_slm=round(cin / max(1, len(slm)), 2),
        ratio_heur=round(cin / max(1, len(heur)), 2),
    )
