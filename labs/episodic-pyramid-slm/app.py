"""
Frontend Gradio — compara heurística Strata vs SLM e mostra taxa de compressão.

  python app.py
"""
from __future__ import annotations

import json

import gradio as gr
from peft import PeftModel

from compressao import comprimir_t1_t2, comprimir_t2_t3
from config import MODELO_CHAT, RUNS, resumo_ambiente
from utils import carregar

_MODELO = None
_TOK = None
_MODO = "(nao carregado)"


def carregar_pipeline():
    global _MODELO, _TOK, _MODO
    if _MODELO is not None:
        return _MODELO, _TOK

    print(f"Carregando base {MODELO_CHAT}...")
    modelo, tok = carregar(MODELO_CHAT)

    adapter_dir = RUNS / "lora_r16_1b7" / "adapter"
    meta_path = RUNS / "lora_r16_1b7" / "meta.json"
    use_lora = adapter_dir.exists()
    if use_lora and meta_path.exists():
        meta = json.loads(meta_path.read_text(encoding="utf-8"))
        if "1.7B" not in str(meta.get("modelo", "")):
            use_lora = False

    if use_lora:
        modelo = PeftModel.from_pretrained(modelo, str(adapter_dir))
        modelo = modelo.merge_and_unload()
        _MODO = "LoRA r16_1b7 (treinado)"
        print(f"Adapter carregado: {adapter_dir}")
    else:
        _MODO = "few-shot 1.7B Instruct (sem LoRA ainda)"
        print(_MODO)

    modelo.eval()
    _MODELO, _TOK = modelo, tok
    return _MODELO, _TOK


def gerar(prompt: str, nivel: str, max_tokens: int, temperatura: float):
    if not prompt.strip():
        return "Cole episodios.", "", "", ""
    modelo, tok = carregar_pipeline()
    if str(nivel).startswith("T2"):
        r = comprimir_t2_t3(modelo, tok, prompt, int(max_tokens), float(temperatura))
    else:
        r = comprimir_t1_t2(modelo, tok, prompt, int(max_tokens), float(temperatura))

    metricas = (
        f"chars entrada     : {r.chars_in}\n"
        f"chars SLM         : {r.chars_out_slm}  (compressao {r.ratio_slm}x)\n"
        f"chars heuristica  : {r.chars_out_heur}  (compressao {r.ratio_heur}x)\n"
        f"modo              : {_MODO}\n"
        f"aviso             : {r.aviso or '(nenhum)'}"
    )
    return r.slm, r.heuristic, metricas, info_gpu()


def info_gpu() -> str:
    return resumo_ambiente() + f"\nModo        : {_MODO}"


EXEMPLO_T1 = """Comprima o cluster de episodios Tier 1 abaixo em um milestone Tier 2.

### Episodio: Generate RSA-256 Keypair for JWT Signing
task_id=task-auth-jwt salience=0.85
outcomes=['RSA keypair generated in PEM format', 'Keys securely stored in vault']
steps:
  - [run_command] Execute openssl genrsa for src/auth/jwt_private.pem files=['src/auth/jwt_private.pem']
  - [run_command] Extract public key to src/auth/jwt_public.pem files=['src/auth/jwt_public.pem']

### Episodio: Deprecate Legacy HS256 HMAC Secret Verification
task_id=task-auth-jwt salience=0.75
outcomes=['HS256 fallback removed', 'All clients migrating to RS256']
steps:
  - [replace_file_content] Remove HS256 secret verification files=['src/auth/verifier.rs']
"""


def main() -> None:
    print(resumo_ambiente())
    carregar_pipeline()

    with gr.Blocks(title="Episodic Pyramid SLM") as demo:
        gr.Markdown(
            "# Episodic Pyramid SLM\n"
            "Comparacao **heuristica Strata** (deterministica) vs **SmolLM2-1.7B**.\n\n"
            "- Heuristica: template `[Milestone Rollup] medoid | N outcomes, M files`\n"
            "- SLM: tenta narrar a decisao (ex.: HS256→RS256) com compressao real de chars"
        )
        with gr.Row():
            with gr.Column():
                prompt = gr.Textbox(label="Prompt / episodios", lines=14, value=EXEMPLO_T1)
                nivel = gr.Radio(
                    ["T1→T2 (milestone)", "T2→T3 (narrativa de sessao)"],
                    value="T1→T2 (milestone)",
                    label="Nivel de compressao",
                )
                max_tokens = gr.Slider(64, 320, value=140, step=16, label="max_new_tokens")
                temperatura = gr.Slider(0.0, 1.0, value=0.2, step=0.1, label="temperatura")
                btn = gr.Button("Comprimir", variant="primary")
            with gr.Column():
                saida_slm = gr.Textbox(label="SLM (resumo narrativo)", lines=8)
                saida_heur = gr.Textbox(label="Heuristica Strata (template)", lines=4)
                metricas = gr.Textbox(label="Compressao (chars in/out)", lines=6)
                gpu = gr.Textbox(label="Ambiente GPU", value=info_gpu(), lines=10)

        btn.click(
            gerar,
            inputs=[prompt, nivel, max_tokens, temperatura],
            outputs=[saida_slm, saida_heur, metricas, gpu],
        )

    demo.launch(server_name="127.0.0.1", server_port=7860)


if __name__ == "__main__":
    main()
