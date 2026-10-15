"""
Roda o sweep de hiperparametros + avaliacoes em sequencia.

  python scripts/sweep.py
"""
from __future__ import annotations

import subprocess
import sys
from pathlib import Path

import _root  # noqa: E402
from _root import ROOT


CMDS = [
    [sys.executable, "scripts/generate_synthetic.py"],
    [sys.executable, "scripts/baseline.py"],
    [sys.executable, "train_lora.py", "--tag", "default", "--r", "16", "--lr", "2e-4", "--epochs", "2"],
    [sys.executable, "scripts/evaluate.py", "--tag", "default"],
    [sys.executable, "train_lora.py", "--tag", "r8_lr1e4", "--r", "8", "--lr", "1e-4", "--epochs", "2"],
    [sys.executable, "scripts/evaluate.py", "--tag", "r8_lr1e4", "--nome", "lora_r8_lr1e4"],
    [sys.executable, "train_lora.py", "--tag", "r16_t23", "--r", "16", "--lr", "2e-4", "--epochs", "2", "--include-t23"],
    [sys.executable, "scripts/evaluate.py", "--tag", "r16_t23", "--nome", "lora_r16_t23"],
    [sys.executable, "scripts/bakeoff.py"],
]


def main() -> None:
    for cmd in CMDS:
        print("\n>>>", " ".join(cmd), flush=True)
        subprocess.run(cmd, check=True, cwd=ROOT)


if __name__ == "__main__":
    main()
