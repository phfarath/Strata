"""
Configuração central do lab Episodic Pyramid SLM.
Todos os scripts importam daqui — não repita caminhos nem hiperparâmetros.
"""
from pathlib import Path
import os

os.environ.setdefault("HF_HUB_DISABLE_SYMLINKS_WARNING", "1")
os.environ.setdefault("TOKENIZERS_PARALLELISM", "false")

# ---------------------------------------------------------------- caminhos
RAIZ = Path(__file__).resolve().parent
DADOS = RAIZ / "data"
RUNS = RAIZ / "runs"
for _p in (DADOS, RUNS):
    _p.mkdir(parents=True, exist_ok=True)

# ---------------------------------------------------------------- hardware
SEED = 42
ATTN = "sdpa"

try:
    import torch

    DEVICE = "cuda" if torch.cuda.is_available() else "cpu"
    if DEVICE == "cuda" and torch.cuda.is_bf16_supported():
        DTYPE = torch.bfloat16
        USA_BF16 = True
    else:
        DTYPE = torch.float32
        USA_BF16 = False
except ImportError:  # scripts de dados (generate_synthetic) não precisam de torch
    torch = None  # type: ignore
    DEVICE = "cpu"
    DTYPE = None
    USA_BF16 = False

# ---------------------------------------------------------------- modelos
MODELO_BASE = "HuggingFaceTB/SmolLM2-135M"
MODELO_CHAT_360 = "HuggingFaceTB/SmolLM2-360M-Instruct"
MODELO_CHAT_1B7 = "HuggingFaceTB/SmolLM2-1.7B-Instruct"
# Inferência/demo de qualidade: 1.7B. Treino acadêmico também roda no 360M.
MODELO_CHAT = MODELO_CHAT_1B7

# ---------------------------------------------------------------- LoRA defaults
LORA_R = 16
LORA_ALPHA = 32
LORA_DROPOUT = 0.05
MAX_SEQ_LEN = 1024
LR = 2e-4
EPOCHS = 2
BATCH_SIZE = 1
GRAD_ACCUM = 8

# ---------------------------------------------------------------- prompts
SISTEMA_T1_T2 = (
    "Você é o compressor de memória episódica do Strata. "
    "Dado um cluster de episódios Tier 1 (traces de execução), "
    "escreva um resumo de milestone Tier 2 em português: "
    "objetivo, arquivos tocados, outcomes e falhas relevantes. "
    "Seja conciso (3 a 6 frases). Se o cluster estiver vazio ou "
    "não for episódico, diga que não sabe."
)

SISTEMA_T2_T3 = (
    "Você é o compressor de memória episódica do Strata. "
    "Dado um conjunto de milestones Tier 2, escreva a narrativa "
    "de sessão Tier 3 em português: o arco da sessão, decisões "
    "principais e riscos remanescentes. Seja conciso (4 a 8 frases)."
)


def resumo_ambiente() -> str:
    if torch is None:
        return "PyTorch      : (não instalado)\nDevice       : cpu"
    linhas = [
        f"PyTorch      : {torch.__version__}",
        f"CUDA         : {torch.version.cuda}",
        f"Device       : {DEVICE}",
        f"dtype        : {DTYPE}",
        f"attn         : {ATTN}",
    ]
    if DEVICE == "cuda":
        p = torch.cuda.get_device_properties(0)
        livre, total = torch.cuda.mem_get_info()
        linhas += [
            f"GPU          : {p.name} (sm_{p.major}{p.minor})",
            f"VRAM total   : {total / 1024**3:.2f} GB",
            f"VRAM livre   : {livre / 1024**3:.2f} GB",
            f"bf16         : {torch.cuda.is_bf16_supported()}",
        ]
    return "\n".join(linhas)


if __name__ == "__main__":
    print(resumo_ambiente())
