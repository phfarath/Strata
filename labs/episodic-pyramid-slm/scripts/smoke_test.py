"""Valida GPU + libs + download do modelo. Precisa terminar com === AMBIENTE OK ===."""
import _root  # noqa: E402  # adiciona raiz ao path
from config import MODELO_CHAT, DEVICE, resumo_ambiente
from utils import carregar, chat, liberar, medir

print(resumo_ambiente())
assert DEVICE == "cuda", (
    "CUDA indisponível. Reinstale PyTorch com:\n"
    "  pip install torch --index-url https://download.pytorch.org/whl/cu128"
)

with medir("smoke load+chat"):
    modelo, tok = carregar(MODELO_CHAT)
    out = chat(modelo, tok, "Diga apenas: ok", max_new_tokens=8)
    print(f"resposta: {out!r}")
    liberar(modelo, tok)

print("=== AMBIENTE OK ===")
