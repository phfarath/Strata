"""Prévia dos dados sintéticos."""
import _root  # noqa: E402  # adiciona raiz ao path
import json
from pathlib import Path
from utils import ler_jsonl

d = Path("data")
print("=== CONTAGENS ===")
for n in ["episodes_t1.jsonl", "sft_t1_to_t2.jsonl", "sft_t2_to_t3.jsonl"]:
    rows = ler_jsonl(d / n)
    print(f"  {n}: {len(rows)}")
g = json.loads((d / "gabarito.json").read_text(encoding="utf-8"))
print(f"  gabarito.json: {len(g)}")

print("\n=== EPISODIO T1 (exemplo) ===")
ep = ler_jsonl(d / "episodes_t1.jsonl")[0]
print(json.dumps(ep, ensure_ascii=False, indent=2)[:1100])

print("\n=== PAR SFT T1->T2 (exemplo) ===")
p = ler_jsonl(d / "sft_t1_to_t2.jsonl")[0]
print("task_id:", p.get("task_id"))
print("prompt:\n", p["prompt"][:400])
print("\ncompletion:\n", p["completion"][:320])

print("\n=== PAR SFT T2->T3 (exemplo) ===")
p2 = ler_jsonl(d / "sft_t2_to_t3.jsonl")[0]
print("prompt:\n", p2["prompt"][:350])
print("\ncompletion:\n", p2["completion"][:320])

print("\n=== GABARITO (amostra) ===")
for item in g[:4]:
    print(f"  #{item['id']} [{item['tipo']}] {item['pergunta'][:85]}...")

print("\n=== HEURISTICA vs GOLD ===")
h = json.loads((d / "heuristic_baselines.json").read_text(encoding="utf-8"))
for tid, row in h.items():
    print(f"\n[{tid}]")
    print("  heur:", row["heuristic"][:100])
    print("  gold:", row["gold"][:120])
