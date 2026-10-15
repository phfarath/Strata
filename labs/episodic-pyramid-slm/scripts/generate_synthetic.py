"""
Gera dados sintéticos da Hierarchical Episodic Memory Pyramid.

Produz:
  data/episodes_t1.jsonl      — episódios Tier 1 (traces)
  data/sft_t1_to_t2.jsonl     — pares treino compressão T1→T2
  data/sft_t2_to_t3.jsonl     — pares treino compressão T2→T3
  data/gabarito.json          — avaliação held-out
  data/heuristic_baselines.json — summaries estilo template Strata

Baseado nos 3 clusters do eval Strata:
  task-auth-jwt, task-sqlite-pool, task-cdc-outbox
"""
from __future__ import annotations

import _root  # noqa: E402  # adiciona raiz ao path
import hashlib
import json
import random
from copy import deepcopy
from pathlib import Path

from config import DADOS, SEED, SISTEMA_T1_T2, SISTEMA_T2_T3


def escrever_jsonl(caminho: Path, rows: list[dict]) -> None:
    caminho.parent.mkdir(parents=True, exist_ok=True)
    caminho.write_text(
        "\n".join(json.dumps(r, ensure_ascii=False) for r in rows) + ("\n" if rows else ""),
        encoding="utf-8",
    )


random.seed(SEED)

# ---------------------------------------------------------------------------
# Templates canônicos (18 episódios base × 3 clusters)
# ---------------------------------------------------------------------------

CLUSTERS = {
    "task-auth-jwt": {
        "agent": "agent-auth-lead",
        "provenance": "Auth team RFC-042 JWT migration",
        "t2_gold": (
            "Milestone Auth JWT: migração de HS256 para RS256 concluída. "
            "Gerou keypair RSA-2048, implementou Claims/token minting, expôs JWKS "
            "em /.well-known/jwks.json, removeu fallback HS256, moveu revogação "
            "para Redis e adicionou middleware Bearer + rate limit 100 req/s. "
            "Arquivos: src/auth/*.rs, src/middleware/*.rs, src/storage/redis.rs. "
            "Outcomes: RS256 operacional, HS256 deprecado, revogação instantânea."
        ),
        "episodes": [
            {
                "summary": "Generate RSA-256 Keypair for JWT Signing",
                "salience": 0.85,
                "outcomes": ["RSA keypair generated in PEM format", "Keys securely stored in vault"],
                "steps": [
                    {"action": "Execute openssl genrsa for src/auth/jwt_private.pem", "tool": "run_command",
                     "files": ["src/auth/jwt_private.pem"]},
                    {"action": "Extract public key to src/auth/jwt_public.pem", "tool": "run_command",
                     "files": ["src/auth/jwt_public.pem"]},
                    {"action": "Verify key integrity with openssl rsa -check", "tool": "run_command",
                     "files": ["src/auth/jwt_private.pem"]},
                ],
            },
            {
                "summary": "Implement Token Minting and Claims Validation",
                "salience": 0.90,
                "outcomes": ["JWT claim structure standardized", "Issuer and audience validated"],
                "steps": [
                    {"action": "Define Claims struct in src/auth/claims.rs", "tool": "replace_file_content",
                     "files": ["src/auth/claims.rs"]},
                    {"action": "Implement sign_token RS256 in src/auth/token.rs", "tool": "replace_file_content",
                     "files": ["src/auth/token.rs"]},
                    {"action": "Run cargo test auth token module", "tool": "run_command", "files": []},
                ],
            },
            {
                "summary": "Add JWKS Endpoint for Asymmetric Key Rotation",
                "salience": 0.80,
                "outcomes": ["JWKS endpoint exposed at /.well-known/jwks.json", "Key ID kid added to header"],
                "steps": [
                    {"action": "Create JWKS serializer in src/auth/jwks.rs", "tool": "write_to_file",
                     "files": ["src/auth/jwks.rs"]},
                    {"action": "Register /.well-known/jwks.json in src/api/routes.rs", "tool": "replace_file_content",
                     "files": ["src/api/routes.rs"]},
                ],
            },
            {
                "summary": "Deprecate Legacy HS256 HMAC Secret Verification",
                "salience": 0.75,
                "outcomes": ["HS256 fallback removed", "All clients migrating to RS256"],
                "steps": [
                    {"action": "Scan src/auth/verifier.rs for HS256", "tool": "run_command",
                     "files": ["src/auth/verifier.rs"]},
                    {"action": "Remove HS256 secret verification", "tool": "replace_file_content",
                     "files": ["src/auth/verifier.rs"]},
                    {"action": "Run auth integration tests", "tool": "run_command", "files": []},
                ],
            },
            {
                "summary": "Migrate Refresh Token Revocation Storage to Redis",
                "salience": 0.85,
                "outcomes": ["Redis cluster store connected for blacklist", "Instant token revocation operational"],
                "steps": [
                    {"action": "Implement Redis pool in src/storage/redis.rs", "tool": "write_to_file",
                     "files": ["src/storage/redis.rs"]},
                    {"action": "Write revocation check with 7-day TTL", "tool": "replace_file_content",
                     "files": ["src/auth/revocation.rs"]},
                ],
            },
            {
                "summary": "Add Middleware for Bearer Token Extraction and Rate Limiting",
                "salience": 0.80,
                "outcomes": ["Axum auth middleware attached", "Rate limit 100 req/s per user"],
                "steps": [
                    {"action": "Create BearerAuth layer in src/middleware/auth.rs", "tool": "write_to_file",
                     "files": ["src/middleware/auth.rs"]},
                    {"action": "Attach token bucket rate limiter", "tool": "replace_file_content",
                     "files": ["src/middleware/rate_limit.rs"]},
                ],
            },
        ],
    },
    "task-sqlite-pool": {
        "agent": "agent-db-lead",
        "provenance": "Database reliability initiative",
        "t2_gold": (
            "Milestone SQLite Pool: confiabilidade sob carga. "
            "Ativou WAL + busy_timeout 5000ms, pool R2D2 de 16 readers, "
            "fila single-writer com Mutex, observou falha de esgotamento "
            "(SQLITE_BUSY_TIMEOUT / DeadlockTimeout) sob 100 writers, "
            "aplicou backoff com jitter e exportou métricas Prometheus. "
            "Arquivos: src/db/*.rs, src/telemetry/prometheus.rs. "
            "Outcomes: WAL ativo, writer serializado, falha documentada, pool recuperado."
        ),
        "episodes": [
            {
                "summary": "Configure SQLite WAL Mode and Busy Timeout Pragma",
                "salience": 0.80,
                "outcomes": ["WAL journal mode enabled", "busy_timeout set to 5000ms"],
                "steps": [
                    {"action": "PRAGMA journal_mode = WAL in src/db/config.rs", "tool": "db_query",
                     "files": ["src/db/config.rs"]},
                    {"action": "PRAGMA busy_timeout = 5000 in src/db/pool.rs", "tool": "db_query",
                     "files": ["src/db/pool.rs"]},
                ],
            },
            {
                "summary": "Implement R2D2 Connection Pool for Reader Connections",
                "salience": 0.85,
                "outcomes": ["Reader connection pool initialized with 16 connections", "Zero read lock contention"],
                "steps": [
                    {"action": "Create R2D2 pool builder in src/db/reader.rs", "tool": "write_to_file",
                     "files": ["src/db/reader.rs"]},
                    {"action": "Benchmark 32-thread concurrent reads", "tool": "run_command", "files": []},
                ],
            },
            {
                "summary": "Introduce Dedicated Single-Writer Mutex Queue",
                "salience": 0.90,
                "outcomes": ["Single-writer serialized via tokio::sync::Mutex", "SQLITE_BUSY write errors prevented"],
                "steps": [
                    {"action": "Wrap writer connection in Mutex in src/db/writer.rs", "tool": "write_to_file",
                     "files": ["src/db/writer.rs"]},
                    {"action": "Implement write queue channel in src/db/queue.rs", "tool": "write_to_file",
                     "files": ["src/db/queue.rs"]},
                ],
            },
            {
                "summary": "Simulate Connection Exhaustion Under Concurrent Load",
                "salience": 0.95,
                "outcomes": ["Pool exhaustion failure observed", "Deadlock timeout in writer lease acquire"],
                "has_failure": True,
                "steps": [
                    {"action": "Spawn 100 concurrent write workers", "tool": "run_command",
                     "files": ["src/db/pool.rs"]},
                    {"action": "Acquire write lease under saturated queue", "tool": "db_query",
                     "files": ["src/db/pool.rs"],
                     "error": "DeadlockTimeout: SQLite connection pool exhausted under concurrent writer load"},
                ],
            },
            {
                "summary": "Tune SQLite Connection Pool Limits and Backoff Policy",
                "salience": 0.85,
                "outcomes": ["Exponential backoff with jitter applied", "Pool recovery verified under load"],
                "steps": [
                    {"action": "Implement jittered backoff in src/db/backoff.rs", "tool": "write_to_file",
                     "files": ["src/db/backoff.rs"]},
                    {"action": "Adjust writer queue capacity to 500", "tool": "replace_file_content",
                     "files": ["src/db/pool.rs"]},
                ],
            },
            {
                "summary": "Add Prometheus Metrics for Pool Wait Time and Active Leases",
                "salience": 0.75,
                "outcomes": ["Metrics sqlite_pool_active_connections exported", "Alert rule for queue depth registered"],
                "steps": [
                    {"action": "Register IntGaugeVec in src/db/metrics.rs", "tool": "write_to_file",
                     "files": ["src/db/metrics.rs"]},
                    {"action": "Hook metrics into prometheus exporter", "tool": "replace_file_content",
                     "files": ["src/telemetry/prometheus.rs"]},
                ],
            },
        ],
    },
    "task-cdc-outbox": {
        "agent": "agent-cdc-lead",
        "provenance": "Event-driven architecture overhaul",
        "t2_gold": (
            "Milestone CDC Outbox: entrega confiável de eventos. "
            "Criou tabela outbox_events, publisher atômico na mesma transação, "
            "poller com SELECT FOR UPDATE SKIP LOCKED (batch 50), "
            "Kafka idempotente com acks=all, DLQ após 3 retries e "
            "métricas de lag. Arquivos: migrations/003_outbox.sql, src/cdc/*.rs. "
            "Outcomes: outbox transacional, entrega idempotente, poison pills isolados."
        ),
        "episodes": [
            {
                "summary": "Create Outbox Events Table with Sequence Number",
                "salience": 0.80,
                "outcomes": ["Table outbox_events created with BIGSERIAL id", "Payload stored as JSONB"],
                "steps": [
                    {"action": "Apply migration migrations/003_outbox.sql", "tool": "db_query",
                     "files": ["migrations/003_outbox.sql"]},
                    {"action": "Define OutboxEvent in src/cdc/schema.rs", "tool": "write_to_file",
                     "files": ["src/cdc/schema.rs"]},
                ],
            },
            {
                "summary": "Implement Transactional Outbox Event Publisher",
                "salience": 0.85,
                "outcomes": ["Domain events committed atomically with entity state", "Publisher helper unit-tested"],
                "steps": [
                    {"action": "Implement publish_event in src/cdc/publisher.rs", "tool": "write_to_file",
                     "files": ["src/cdc/publisher.rs"]},
                    {"action": "Add domain events in src/cdc/events.rs", "tool": "replace_file_content",
                     "files": ["src/cdc/events.rs"]},
                ],
            },
            {
                "summary": "Implement Polling CDC Engine with Batch Fetch",
                "salience": 0.80,
                "outcomes": ["Poll loop fetches 50 pending events per tick", "Row-level locking with SKIP LOCKED"],
                "steps": [
                    {"action": "Batch query with SKIP LOCKED in src/cdc/poller.rs", "tool": "write_to_file",
                     "files": ["src/cdc/poller.rs"]},
                    {"action": "Add batch processor in src/cdc/batch.rs", "tool": "write_to_file",
                     "files": ["src/cdc/batch.rs"]},
                ],
            },
            {
                "summary": "Add Idempotent Kafka Delivery with Ack Receipts",
                "salience": 0.90,
                "outcomes": ["Kafka producer configured with acks=all", "Deduplication key set to event UUID"],
                "steps": [
                    {"action": "Configure rdkafka producer in src/cdc/kafka.rs", "tool": "write_to_file",
                     "files": ["src/cdc/kafka.rs"]},
                    {"action": "Send batch to topic orders.v1 and verify acks", "tool": "run_command",
                     "files": ["src/cdc/ack.rs"]},
                ],
            },
            {
                "summary": "Implement Dead Letter Queue for Poison Pill Events",
                "salience": 0.85,
                "outcomes": ["DLQ routing active after 3 failed retries", "Poison pill isolation confirmed"],
                "steps": [
                    {"action": "Create DLQ table and router in src/cdc/dlq.rs", "tool": "write_to_file",
                     "files": ["src/cdc/dlq.rs"]},
                    {"action": "Wire retry policy max_attempts=3", "tool": "replace_file_content",
                     "files": ["src/cdc/retry.rs"]},
                ],
            },
            {
                "summary": "Export CDC Lag and Throughput Metrics",
                "salience": 0.70,
                "outcomes": ["cdc_outbox_lag_seconds exported", "Throughput dashboard created"],
                "steps": [
                    {"action": "Register lag histogram in src/cdc/metrics.rs", "tool": "write_to_file",
                     "files": ["src/cdc/metrics.rs"]},
                    {"action": "Add Grafana panel for outbox lag", "tool": "replace_file_content",
                     "files": ["ops/grafana/cdc.json"]},
                ],
            },
        ],
    },
}

# Variações para expandir o dataset (~4 variantes por episódio base)
VARIANT_SUFFIXES = [
    "",
    " (retry after review)",
    " - follow-up polish",
    " with extra logging",
    " (post-mortem notes)",
    " after peer review",
]

T3_GOLD = (
    "Narrativa de sessão: a equipe estabilizou autenticação (JWT RS256 + JWKS + Redis), "
    "endureceu o pool SQLite após esgotamento sob carga concorrente, e entregou o pipeline "
    "CDC outbox com Kafka idempotente e DLQ. Decisões-chave: assimetria de chaves, "
    "single-writer serializado, SKIP LOCKED no poller. Riscos remanescentes: migração "
    "completa dos clientes HS256, alertas de profundidade da fila writer, e monitoramento "
    "de lag do outbox em produção."
)


def _uid(prefix: str, *parts) -> str:
    dig = hashlib.sha256("|".join(map(str, parts)).encode()).hexdigest()[:16]
    return f"{prefix}-{dig}"


def _files_from_steps(steps: list[dict]) -> list[str]:
    seen, out = set(), []
    for s in steps:
        for f in s.get("files", []):
            if f not in seen:
                seen.add(f)
                out.append(f)
    return out


def heuristic_summary(episodes: list[dict]) -> str:
    """Réplica do template Strata fuse_causal_invariants."""
    if not episodes:
        return "[Milestone Rollup] Empty Cluster | Invariants: 0 outcomes, 0 files touched"
    medoid = max(episodes, key=lambda e: (e["salience"], e["id"]))
    outcomes = []
    seen = set()
    for e in episodes:
        for o in e["outcomes"]:
            if o not in seen:
                seen.add(o)
                outcomes.append(o)
    files = []
    seen_f = set()
    for e in episodes:
        for f in _files_from_steps(e["steps"]):
            if f not in seen_f:
                seen_f.add(f)
                files.append(f)
    return (
        f"[Milestone Rollup] {medoid['summary']} | "
        f"Invariants: {len(outcomes)} outcomes, {len(files)} files touched"
    )


def format_cluster_prompt(episodes: list[dict]) -> str:
    blocos = []
    for e in episodes:
        steps_txt = "\n".join(
            f"  - [{s['tool']}] {s['action']}"
            + (f" ERR={s['error']}" if s.get("error") else "")
            + (f" files={s['files']}" if s.get("files") else "")
            for s in e["steps"]
        )
        blocos.append(
            f"### Episódio: {e['summary']}\n"
            f"task_id={e['task_id']} salience={e['salience']:.2f}\n"
            f"outcomes={e['outcomes']}\n"
            f"steps:\n{steps_txt}"
        )
    return (
        "Comprima o cluster de episódios Tier 1 abaixo em um milestone Tier 2.\n\n"
        + "\n\n".join(blocos)
    )


def format_t2_prompt(milestones: list[dict]) -> str:
    blocos = [
        f"### Milestone ({m['task_id']}):\n{m['summary']}"
        for m in milestones
    ]
    return (
        "Comprima os milestones Tier 2 abaixo em uma narrativa de sessão Tier 3.\n\n"
        + "\n\n".join(blocos)
    )


def build_episodes(n_variants: int = 6) -> list[dict]:
    episodes = []
    base_time = 1_700_000_000
    idx = 0
    for task_id, cluster in CLUSTERS.items():
        for vi in range(n_variants):
            for ei, tmpl in enumerate(cluster["episodes"]):
                ep = deepcopy(tmpl)
                suffix = VARIANT_SUFFIXES[vi % len(VARIANT_SUFFIXES)]
                if suffix:
                    ep["summary"] = ep["summary"] + suffix
                    # leve ruído nos outcomes
                    ep["outcomes"] = [
                        o if random.random() > 0.3 else o + " (verified)"
                        for o in ep["outcomes"]
                    ]
                ep.update(
                    {
                        "id": _uid("ep", task_id, vi, ei, ep["summary"]),
                        "session_id": f"sess-synth-{vi}",
                        "agent_id": cluster["agent"],
                        "task_id": task_id,
                        "tier": 1,
                        "provenance": cluster["provenance"],
                        "created_at": base_time + idx * 60,
                    }
                )
                episodes.append(ep)
                idx += 1
    return episodes


GOLD_PARAPHRASES = {
    "task-auth-jwt": [
        CLUSTERS["task-auth-jwt"]["t2_gold"],
        (
            "Tier 2 Auth JWT: troca de HMAC HS256 por assinatura assimétrica RS256. "
            "Inclui keypair PEM, Claims, JWKS público, remoção do fallback HS256, "
            "blacklist Redis e middleware Bearer com rate limit. "
            "Invariantes: RS256 ativo, HS256 removido, revogação online."
        ),
        (
            "Milestone: autenticação migrada para RS256/JWKS. "
            "Arquivos principais em src/auth e src/middleware. "
            "Resultado: tokens assimétricos, kids no header, revogação via Redis."
        ),
    ],
    "task-sqlite-pool": [
        CLUSTERS["task-sqlite-pool"]["t2_gold"],
        (
            "Tier 2 SQLite: pool endurecido com WAL, readers R2D2 e writer único. "
            "Falha crítica preservada: DeadlockTimeout / SQLITE_BUSY sob 100 writers. "
            "Mitigações: backoff com jitter e métricas Prometheus de fila/espera."
        ),
        (
            "Milestone pool SQLite: confiabilidade sob contenção. "
            "Documenta esgotamento do pool e recuperação com backoff. "
            "Invariantes: single-writer, WAL, alertas de profundidade da fila."
        ),
    ],
    "task-cdc-outbox": [
        CLUSTERS["task-cdc-outbox"]["t2_gold"],
        (
            "Tier 2 CDC: outbox transacional + poller SKIP LOCKED + Kafka idempotente. "
            "DLQ após 3 falhas isola poison pills. "
            "Invariantes: publish atômico, acks=all, lag observável."
        ),
        (
            "Milestone outbox/CDC: eventos confiáveis ponta a ponta. "
            "Tabela outbox_events, batch 50, Kafka com dedupe por UUID, DLQ. "
            "Arquivos em migrations/003_outbox.sql e src/cdc."
        ),
    ],
}


def build_sft_t1_t2(episodes: list[dict]) -> list[dict]:
    pairs = []
    by_session_task: dict[tuple[str, str], list[dict]] = {}
    for e in episodes:
        key = (e["session_id"], e["task_id"])
        by_session_task.setdefault(key, []).append(e)

    for (session_id, task_id), cluster in by_session_task.items():
        paraphrases = GOLD_PARAPHRASES[task_id]
        for pi, gold in enumerate(paraphrases):
            pairs.append(
                {
                    "id": _uid("sft12", session_id, task_id, "full", pi),
                    "task_id": task_id,
                    "session_id": session_id,
                    "sistema": SISTEMA_T1_T2,
                    "prompt": format_cluster_prompt(cluster),
                    "completion": gold,
                    "nivel": "t1_to_t2",
                }
            )
        # sub-clusters (metade / terço) para diversificar
        if len(cluster) >= 4:
            half = cluster[: len(cluster) // 2]
            pairs.append(
                {
                    "id": _uid("sft12", session_id, task_id, "half"),
                    "task_id": task_id,
                    "session_id": session_id,
                    "sistema": SISTEMA_T1_T2,
                    "prompt": format_cluster_prompt(half),
                    "completion": (
                        paraphrases[0].split(".")[0] + ". "
                        "Resumo parcial do cluster (primeiros episódios). "
                        "Arquivos e outcomes ainda em progresso."
                    ),
                    "nivel": "t1_to_t2",
                }
            )
            third = cluster[len(cluster) // 3 :]
            pairs.append(
                {
                    "id": _uid("sft12", session_id, task_id, "tail"),
                    "task_id": task_id,
                    "session_id": session_id,
                    "sistema": SISTEMA_T1_T2,
                    "prompt": format_cluster_prompt(third),
                    "completion": paraphrases[1],
                    "nivel": "t1_to_t2",
                }
            )

    # Pares de abstenção (armadilhas) — ensina a dizer "não sei"
    traps = [
        "Comprima o cluster de migração PostgreSQL sharding com Citus.",
        "Resuma o milestone Tier 2 do task-graphql-federation.",
        "Liste episódios Tier 1 da migração monorepo para Bazel remoto.",
        "Comprima traces de deploy Kubernetes blue-green multi-região.",
    ]
    for i, trap in enumerate(traps):
        pairs.append(
            {
                "id": _uid("sft12", "trap", i),
                "task_id": None,
                "session_id": "sess-trap",
                "sistema": SISTEMA_T1_T2,
                "prompt": trap,
                "completion": "Não sei — esse cluster não está no material episódico treinado.",
                "nivel": "t1_to_t2",
            }
        )
    return pairs


def build_sft_t2_t3(n_sessions: int = 4) -> list[dict]:
    pairs = []
    for vi in range(n_sessions):
        milestones = [
            {
                "task_id": tid,
                "summary": CLUSTERS[tid]["t2_gold"],
            }
            for tid in CLUSTERS
        ]
        # às vezes omitir um cluster
        if vi % 2 == 1:
            milestones = milestones[:2]
            completion = (
                "Narrativa parcial: autenticação JWT RS256 e pool SQLite estabilizados. "
                "CDC outbox ainda não consolidado nesta sessão. "
                "Risco: entrega de eventos sem outbox transacional."
            )
        else:
            completion = T3_GOLD
        pairs.append(
            {
                "id": _uid("sft23", vi),
                "session_id": f"sess-synth-{vi}",
                "sistema": SISTEMA_T2_T3,
                "prompt": format_t2_prompt(milestones),
                "completion": completion,
                "nivel": "t2_to_t3",
            }
        )
    return pairs


def build_gabarito() -> list[dict]:
    """Held-out: NÃO entra no treino. Mistura factual / síntese / armadilha."""
    return [
        {
            "id": 1,
            "tipo": "factual",
            "nivel": "t1_to_t2",
            "task_id": "task-auth-jwt",
            "pergunta": (
                "Comprima: episódios Auth JWT geraram keypair RSA, Claims RS256, "
                "JWKS em /.well-known/jwks.json, removeram HS256 e usaram Redis para revogação. "
                "Qual o milestone Tier 2?"
            ),
            "resposta_esperada": CLUSTERS["task-auth-jwt"]["t2_gold"],
            "palavras_chave": ["RS256", "JWKS", "HS256", "Redis"],
        },
        {
            "id": 2,
            "tipo": "factual",
            "nivel": "t1_to_t2",
            "task_id": "task-sqlite-pool",
            "pergunta": (
                "Comprima: WAL + busy_timeout, pool R2D2 16 readers, single-writer Mutex, "
                "falha DeadlockTimeout sob 100 writers, backoff jitter, métricas Prometheus. "
                "Qual o milestone Tier 2?"
            ),
            "resposta_esperada": CLUSTERS["task-sqlite-pool"]["t2_gold"],
            "palavras_chave": ["WAL", "DeadlockTimeout", "backoff", "Prometheus"],
        },
        {
            "id": 3,
            "tipo": "factual",
            "nivel": "t1_to_t2",
            "task_id": "task-cdc-outbox",
            "pergunta": (
                "Comprima: outbox_events, publisher atômico, SKIP LOCKED batch 50, "
                "Kafka acks=all, DLQ após 3 retries. Qual o milestone Tier 2?"
            ),
            "resposta_esperada": CLUSTERS["task-cdc-outbox"]["t2_gold"],
            "palavras_chave": ["outbox", "SKIP LOCKED", "Kafka", "DLQ"],
        },
        {
            "id": 4,
            "tipo": "sintese",
            "nivel": "t1_to_t2",
            "task_id": "task-sqlite-pool",
            "pergunta": (
                "No cluster SQLite, qual falha crítica deve aparecer no milestone Tier 2 "
                "e por que ela importa para a pirâmide episódica?"
            ),
            "resposta_esperada": (
                "A falha DeadlockTimeout / SQLITE_BUSY sob carga de writers deve ser "
                "preservada no rollup porque a pirâmide comprime sem apagar invariantes "
                "causais; o drill-down precisa reencontrar o episódio de esgotamento."
            ),
            "palavras_chave": ["DeadlockTimeout", "SQLITE_BUSY", "invariante", "drill-down"],
        },
        {
            "id": 5,
            "tipo": "sintese",
            "nivel": "t2_to_t3",
            "task_id": None,
            "pergunta": (
                "Dados os três milestones (Auth JWT RS256, SQLite pool com falha de carga, "
                "CDC outbox Kafka), escreva a narrativa Tier 3 da sessão."
            ),
            "resposta_esperada": T3_GOLD,
            "palavras_chave": ["JWT", "SQLite", "outbox", "risco"],
        },
        {
            "id": 6,
            "tipo": "sintese",
            "nivel": "t1_to_t2",
            "task_id": "task-auth-jwt",
            "pergunta": (
                "Por que o template heurístico '[Milestone Rollup] <medoid> | Invariants: N outcomes, M files' "
                "é insuficiente comparado a um resumo gerado por SLM?"
            ),
            "resposta_esperada": (
                "O template só ecoa o medoid e conta outcomes/arquivos; não narra a decisão "
                "(HS256→RS256), nem a falha causal, nem o arco do cluster. O SLM comprime "
                "semântica preservando invariantes legíveis para zoom_out."
            ),
            "palavras_chave": ["medoid", "template", "semântica", "invariante"],
        },
        {
            "id": 7,
            "tipo": "armadilha",
            "nivel": "t1_to_t2",
            "task_id": None,
            "pergunta": (
                "Comprima o cluster de episódios sobre migração PostgreSQL sharding "
                "com Citus e rebalanceamento online de shards."
            ),
            "resposta_esperada": "Não sei — esse cluster não está no material episódico treinado.",
            "palavras_chave": ["não sei"],
        },
        {
            "id": 8,
            "tipo": "armadilha",
            "nivel": "t1_to_t2",
            "task_id": None,
            "pergunta": "Resuma o milestone Tier 2 do task-graphql-federation (não existe nos dados).",
            "resposta_esperada": "Não sei — task_id ausente no corpus.",
            "palavras_chave": ["não sei"],
        },
        {
            "id": 9,
            "tipo": "factual",
            "nivel": "t1_to_t2",
            "task_id": "task-auth-jwt",
            "pergunta": "Quais arquivos de auth aparecem tipicamente no rollup JWT?",
            "resposta_esperada": (
                "src/auth/claims.rs, token.rs, jwks.rs, verifier.rs, revocation.rs "
                "e middleware de Bearer/rate limit."
            ),
            "palavras_chave": ["claims.rs", "jwks", "verifier", "middleware"],
        },
        {
            "id": 10,
            "tipo": "factual",
            "nivel": "t1_to_t2",
            "task_id": "task-cdc-outbox",
            "pergunta": "Qual o benefício do SKIP LOCKED no poller CDC segundo os episódios?",
            "resposta_esperada": (
                "Zero contenção de lock entre workers paralelos ao buscar eventos pending."
            ),
            "palavras_chave": ["SKIP LOCKED", "contenção", "workers"],
        },
        {
            "id": 11,
            "tipo": "sintese",
            "nivel": "t2_to_t3",
            "task_id": None,
            "pergunta": (
                "Na pirâmide, o que muda ao comprimir Tier 2 → Tier 3 versus Tier 1 → Tier 2?"
            ),
            "resposta_esperada": (
                "T1→T2 agrega traces de uma task em milestone; T2→T3 agrega milestones "
                "de várias tasks na narrativa de sessão, com decisões e riscos globais."
            ),
            "palavras_chave": ["milestone", "sessão", "narrativa", "risco"],
        },
        {
            "id": 12,
            "tipo": "armadilha",
            "nivel": "t1_to_t2",
            "task_id": None,
            "pergunta": "Liste os episódios Tier 1 da migração de monorepo para Bazel remoto.",
            "resposta_esperada": "Não sei — fora do corpus sintético da pyramid.",
            "palavras_chave": ["não sei"],
        },
    ]


def build_heuristic_baselines(episodes: list[dict]) -> dict:
    out = {}
    for task_id in CLUSTERS:
        cluster = [e for e in episodes if e["task_id"] == task_id and e["session_id"] == "sess-synth-0"]
        out[task_id] = {
            "heuristic": heuristic_summary(cluster),
            "gold": CLUSTERS[task_id]["t2_gold"],
            "n_episodes": len(cluster),
        }
    return out


def main() -> None:
    DADOS.mkdir(parents=True, exist_ok=True)

    episodes = build_episodes(n_variants=6)
    sft12 = build_sft_t1_t2(episodes)
    sft23 = build_sft_t2_t3(n_sessions=4)
    gabarito = build_gabarito()
    heuristics = build_heuristic_baselines(episodes)

    # Higiene: nenhuma pergunta do gabarito no treino (prefixo 40 chars)
    gab_pref = {g["pergunta"].lower()[:40] for g in gabarito}
    antes = len(sft12)
    sft12 = [p for p in sft12 if p["prompt"].lower()[:40] not in gab_pref]
    print(f"Removidos {antes - len(sft12)} pares T1->T2 que vazavam gabarito (prefixo).")

    escrever_jsonl(DADOS / "episodes_t1.jsonl", episodes)
    escrever_jsonl(DADOS / "sft_t1_to_t2.jsonl", sft12)
    escrever_jsonl(DADOS / "sft_t2_to_t3.jsonl", sft23)
    (DADOS / "gabarito.json").write_text(
        json.dumps(gabarito, ensure_ascii=False, indent=2), encoding="utf-8"
    )
    (DADOS / "heuristic_baselines.json").write_text(
        json.dumps(heuristics, ensure_ascii=False, indent=2), encoding="utf-8"
    )

    print("=== DADOS SINTÉTICOS ===")
    print(f"  episodes_t1     : {len(episodes)}")
    print(f"  sft_t1_to_t2    : {len(sft12)}")
    print(f"  sft_t2_to_t3    : {len(sft23)}")
    print(f"  gabarito        : {len(gabarito)}")
    print(f"  heurísticas     : {len(heuristics)} clusters")
    print(f"  pasta           : {DADOS}")


if __name__ == "__main__":
    main()
