"""Baseline: SmolLM2 Instruct SEM fine-tune no gabarito da pyramid."""
import _root  # noqa: E402  # adiciona raiz ao path
from config import MODELO_CHAT, SISTEMA_T1_T2, resumo_ambiente
from avaliacao import avaliar_rota
from utils import carregar, chat, fixar_seed, liberar, medir, salvar_resultado

fixar_seed()
print(resumo_ambiente())

modelo, tok = carregar(MODELO_CHAT)


def responder(pergunta: str) -> str:
    return chat(modelo, tok, pergunta, sistema=SISTEMA_T1_T2, max_new_tokens=220)


with medir("baseline eval"):
    resultado = avaliar_rota("baseline", responder)

salvar_resultado("baseline", resultado)
liberar(modelo, tok)
