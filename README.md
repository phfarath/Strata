# Strata — Portable Persistent Memory Layer & Cognitive Runtime

[![Rust](https://img.shields.io/badge/rust-2021_edition-orange.svg)](https://www.rust-lang.org/)
[![Tests](https://img.shields.io/badge/tests-49%2F49%20passing%20(100%25)-brightgreen.svg)]()
[![MCP](https://img.shields.io/badge/MCP-2024--11--05%20%7C%202025--11--25%20%7C%202026--07--28-blue.svg)](https://modelcontextprotocol.io/)
[![Storage](https://img.shields.io/badge/storage-SQLite%20Offline--First%20%2B%20FTS5-blueviolet.svg)](https://www.sqlite.org/)
[![License](https://img.shields.io/badge/license-MIT%20%2F%20Apache--2.0-blue.svg)]()

> **Strata** é uma camada de memória persistente portável e runtime cognitivo em Rust projetada para eliminar a amnésia crônica e o isolamento de contexto entre agentes de código (**Cursor IDE**, **Claude Code**, **Codex CLI**, **Gemini CLI** e **Antigravity**).

Em vez de atuar como um mero wrapper de embeddings, o Strata funciona como um **hipocampo externo determinístico**, combinando fundamentos da ciência cognitiva (**ACT-R**, **Curva de Retenção de Ebbinghaus** e **JTMS - Truth Maintenance System**) com armazenamento **SQLite local (Offline-First)**, replicação **CDC (Change Data Capture)**, síntese automatizada de datasets de alinhamento (**DPO / KTO / SFT**) e transporte universal **MCP (Model Context Protocol)**.

---

```text
┌──────────────────────────────────────────────────────────────────────────────────────────────────┐
│                                 STRATA COGNITIVE RUNTIME (RUST)                                 │
└────────────────────────────────────────────────┬─────────────────────────────────────────────────┘
                                                 │
    ┌────────────────────────────────────────────┼────────────────────────────────────────────┐
    ▼                                            ▼                                            ▼
┌──────────────────────────────┐ ┌──────────────────────────────┐ ┌──────────────────────────────┐
│     1. TRANSPORTE MCP        │ │   2. HOOKS DETERMINÍSTICOS   │ │  3. COMPILADOR MULTI-HOST    │
│  • JSON-RPC 2.0 Stdio        │ │  • Injeção preempitiva       │ │  • Injeção não-destrutiva de │
│  • Compatível com:           │ │    (< 50 tokens no prompt)   │ │    regras compiladas em:     │
│    2024-11-05, 2025-11-25    │ │  • Captura silenciosa de     │ │    - .cursor/rules/*.mdc     │
│    e 2026-07-28 (Stateless)  │ │    erros e anti-padrões      │ │    - CLAUDE.md               │
│  • 5 Ferramentas de Memória: │ │  • Ciclo de Sessão:          │ │    - AGENTS.md               │
│    search, get, write,       │ │    SessionStart, SessionEnd, │ │    - .gemini/GEMINI.md       │
│    digest, memory_feedback   │ │    UserPrompt, PostTool      │ │  • Context Budgeting estrito │
└──────────────────────────────┘ └──────────────────────────────┘ └──────────────────────────────┘
                                                 │
                                                 ▼
┌──────────────────────────────────────────────────────────────────────────────────────────────────┐
│                            MOTOR COGNITIVO & PERSISTÊNCIA OFFLINE                                │
│  • ACT-R Base-Level Activation: Am = α·ln(∑ t_k^-d) + β·Im + γ·Cm                                │
│  • Ebbinghaus Retention: Rm(t) = exp(-t / Sm) com ajuste de estabilidade por reforço             │
│  • JTMS (Justification-Based Truth Maintenance): Resolução de contradições e versionamento       │
│  • SQLite + FTS5 BM25 + Embeddings Vetoriais (FastEmbed) com Reciprocal Rank Fusion (RRF)       │
│  • Sincronização CDC (Change Data Capture) com Sequence Monotônico e Retry com Backoff           │
│  • Mineração de Preferência: Exportação DPO / KTO / SFT compatível com TRL, Unsloth e Axolotl    │
└──────────────────────────────────────────────────────────────────────────────────────────────────┘
```

---

## ✨ Principais Capacidades

- 🧠 **Memória Compartilhada Inter-Agentes**: Um fato gravado pelo Claude Code é instantaneamente recuperado pelo Cursor, Codex ou Antigravity sem duplicar arquivos ou alucinar contexto.
- ⚡ **Desempenho Extremo em Rust**: Busca híbrida e compilação de contexto em **< 10ms**, consumo resident de memória no modo daemon **< 10MB RAM**.
- 🛡️ **Captura Silenciosa de Erros (Anti-Patterns)**: Intercepta falhas de compilação e comandos com erro fora da janela de contexto do LLM e injeta avisos cirúrgicos antes de operações subsequentes.
- 🔄 **Transporte MCP Universal Multi-Versão**: Suporta negociação formal com especificações `2024-11-05`, `2025-03-26`, `2025-06-18`, `2025-11-25` (Latest Stable) e modo stateless direto da spec `2026-07-28`.
- 📊 **Síntese de Datasets DPO / KTO / SFT**: Transforma o histórico de tentativas de código em pares de preferência de alta qualidade (`chosen` vs `rejected`) para fine-tuning de modelos locais.
- 📦 **Compilador Multi-Host com Orçamento de Contexto**: Seleciona as top-K memórias por saliência cognitiva e injeta regras nos formatos específicos de cada IDE sem estourar limites de tokens.
- 🔌 **100% Offline-First**: Opera localmente em `~/.strata/strata.db` sem necessidade de conexão com internet ou dependências de servidores centralizados.

---

## 🏛️ Fundamentos Teóricos e Modelagem Cognitiva

### 1. Ativação de Nível-Base (ACT-R)
A ativação de um chunk de memória $m$ quantifica a sua disponibilidade cognitiva com base na frequência e recência de acessos:

$$A_m = \alpha \ln\left(\sum_{k=1}^n t_k^{-d}\right) + \beta I_m + \gamma C_m + \lambda F_m$$

- $t_k$: Tempo decorrido desde o $k$-ésimo acesso ao chunk.
- $d$: Coeficiente de decaimento temporal de esquecimento (default: $0.5$).
- $I_m$: Peso de importância intrínseca atribuído pelo usuário ou pipeline.
- $C_m$: Pontuação de confiança factual validada pelo sistema.
- $F_m$: Acumulador de feedback e reforço cognitivo derivado de sinais implícitos/explícitos.

### 2. Retenção de Ebbinghaus e Estabilidade ($S_m$)
A retenção percentual ao longo do tempo $t$ segue a curva exponencial de Ebbinghaus:

$$R_m(t) = \exp\left(-\frac{t}{S_m}\right)$$

Onde a estabilidade $S_m$ é ampliada incrementalmente através de repetições espaçadas bem-sucedidas e feedbacks positivos:

$$S_m = S_0 \cdot \left(1 + \lambda \ln(u + 1) + \mu I_m\right) + \eta \sum_{s \in \mathcal{S}} w_s r_s$$

### 3. Justification-Based Truth Maintenance System (JTMS)
O Strata mantém consistência lógica estrita entre fatos semânticos. Quando um novo fato entra em contradição com uma diretriz anterior (ex: migração de API REST para gRPC), o JTMS:
1. Detecta o conflito via similaridade semântica ($> 0.85$) associada a marcadores léxicos de contradição.
2. Transita o status do fato antigo para `Deprecated` (nó `OUT`).
3. Aponta o ponteiro `replaced_by` para o novo fato (nó `IN`).
4. Incrementa o número de versão do registro atômico.

---

## 📦 Estrutura do Workspace Cargo

```text
mem-research/
├── crates/
│   ├── strata-core/         # Tipos fundamentais, schemas (Episodic, Semantic, Procedural, CDC, Signals, DPO/KTO)
│   ├── strata-memory/       # SQLite store, FTS5 BM25, FastEmbed, Decay ACT-R, JTMS, SyncEngine, PreferenceMiner, MultiHostCompiler
│   ├── strata-reasoning/    # Adaptadores LLM (OpenRouter free tier, OpenAI, Mock), Prompts de Destilação e Arbitragem JTMS
│   ├── strata-tools/        # Gateway de execução segura, permissões, rate-limiting e captura silenciosa de erros
│   ├── strata-cli/          # Binário CLI unificado (init, mcp, hook, search, doctor, write, get, digest, consolidate, prune, sync, daemon, export, sync-hosts, feedback)
│   └── strata-evals/        # Suíte de avaliação determinística (8 cenários + live OpenRouter integration)
```

---

## 🚀 Instalação e Compilação

### Pré-requisitos
- [Rust 1.80+](https://rustup.rs/) (Edição 2021)
- SQLite 3 (embutido via `rusqlite` bundled)

### Compilar a Versão Release Otimizada
```bash
git clone https://github.com/phfarath/Strata.git
cd Strata

# Compilação otimizada em release
cargo build --release -p strata-cli

# O binário nativo estará disponível em:
# ./target/release/strata-cli.exe (Windows) ou ./target/release/strata-cli (Linux/macOS)
```

### Executar a Suíte de Testes
```bash
cargo test --workspace
```
*Garante 100% de aprovação (49/49 testes) em menos de 0.3 segundos.*

---

## 🔌 Configuração nos Hosts e IDEs

### 1. Claude Code CLI
Adicione o servidor MCP globalmente com o comando:
```bash
claude mcp add -s user strata -- C:/Dev/mem-research/target/release/strata-cli.exe mcp
```

### 2. Cursor IDE
Adicione nas configurações do Cursor (`Settings` $\to$ `Features` $\to$ `MCP` $\to$ `Add New MCP Server`):
- **Name**: `strata`
- **Type**: `command`
- **Command**: `C:/Dev/mem-research/target/release/strata-cli.exe mcp`

Ou crie/atualize `.cursor/mcp.json`:
```json
{
  "mcpServers": {
    "strata": {
      "command": "C:/Dev/mem-research/target/release/strata-cli.exe",
      "args": ["mcp"]
    }
  }
}
```

### 3. Codex CLI & Gemini CLI
O Strata sincroniza automaticamente as regras e diretrizes comportamentais em:
- `AGENTS.md` (Codex / Padrão Universal)
- `.gemini/GEMINI.md` (Gemini CLI)

---

## 🛠️ Guia de Comandos da CLI (`strata`)

| Comando | Descrição | Exemplo de Uso |
|---|---|---|
| `strata doctor` | Diagnóstico completo de integridade do banco SQLite e dos hosts | `strata doctor` |
| `strata mcp` | Inicia o servidor MCP stdio JSON-RPC (2024/2025/2026) | `strata mcp` |
| `strata write` | Grava uma memória persistente (semântica, episódica, skill, anti-pattern) | `strata write "Diretriz X" --summary "Título" --importance 0.9 --tags "arch,db"` |
| `strata search` | Busca híbrida (BM25 + vetorial FastEmbed) com ranking RRF | `strata search "Simplicity"` |
| `strata get` | Recupera os detalhes completos e metadados de uma memória por UUID | `strata get <UUID>` |
| `strata digest` | Gera um resumo de contexto compactado (~300-500 tokens) para bootstrap | `strata digest --tokens 500` |
| `strata feedback` | Aplica reforço cognitivo explícito ajustando importância e estabilidade | `strata feedback --id <UUID> --rating positive --comment "Excelente"` |
| `strata sync-hosts` | Compila e injeta determinísticamente as top memórias nos arquivos de instrução | `strata sync-hosts --target all --budget 1000` |
| `strata export` | Minera e exporta datasets de alinhamento (`dpo`, `kto`, `sft`, `jsonl`, `markdown`) | `strata export --format dpo --out dataset_dpo.jsonl` |
| `strata sync` | Sincronização delta CDC offline-first com endpoint remoto (`push`, `pull`, `status`) | `strata sync status` |
| `strata daemon` | Executa o daemon de sincronização em background (< 10MB RAM) | `strata daemon --interval 30` |
| `strata consolidate`| Executa a destilação de eventos episódicos via LLM em fatos e habilidades | `strata consolidate --all` |
| `strata prune` | Executa o motor de decaimento matemático ACT-R para podar memórias expiradas | `strata prune --threshold 0.2` |

---

## 📊 Mineração de Datasets para Fine-Tuning (DPO / KTO / SFT)

O Strata transforma a experiência de codificação dos agentes em datasets de alinhamento:

```bash
# Exportar pares de preferência DPO (chosen vs rejected)
strata export --format dpo --out dpo_pairs.jsonl

# Exportar amostras binárias KTO
strata export --format kto --out kto_samples.jsonl

# Exportar habilidades procedurais em formato SFT (instruction/input/output)
strata export --format sft --out sft_skills.jsonl
```

### Exemplo de Registro DPO Mined
```json
{
  "id": "5a32f7e8-c034-4c93-ab4a-87e306ea075b",
  "prompt": "Context: Agent encountered obstacle.\nProblem / Trigger: cargo_test_failure\nTask: Execute cargo test suite",
  "chosen": "Mitigation Strategy:\nAvoid repeating identical invalid parameters or unverified flags\nDetails: cargo test -p correct-package-name",
  "rejected": "Anti-pattern approach leading to error 'ToolExecutionError': error: package ID specification 'wrong-package-name' did not match any packages",
  "source_session_id": "failure-pattern",
  "created_at": "2026-08-18T13:59:06Z"
}
```

---

## 🧪 Suíte de Avaliações e Métricas de Validação (`strata-evals`)

O Strata inclui 8 cenários determinísticos de avaliação contínua em [`crates/strata-evals`](crates/strata-evals):
1. `silent_failure_avoidance`: Valida a interceptação de erros out-of-band e emissão de alertas prévios de anti-padrões (< 50 tokens).
2. `cross_host_transfer`: Valida a persistência e recuperação cruzada entre hosts distintos.
3. `decay_curve_simulation`: Valida a precisão matemática das curvas ACT-R e Ebbinghaus em intervalos simulados de 1 hora a 30 dias.
4. `jtms_belief_revision`: Valida a identificação de contradições lógicas, depreciação atômica e versionamento.
5. `procedural_skill_distillation`: Valida a extração de fluxos de recuperação em passos procedurais tipados.
6. `mcp_protocol_multi_version`: Valida o transporte stdio JSON-RPC nas versões 2024-11-05, 2025-11-25 e 2026-07-28 (stateless), além das 5 ferramentas de memória.
7. `offline_first_cdc_sync`: Valida o acúmulo em `sync_outbox`, retry com backoff exponencial e reconciliação JTMS multi-host.
8. `cognitive_feedback_and_alignment`: Valida o pipeline completo de sinais implícitos/explícitos, mineração DPO/KTO/SFT e compilação multi-host com orçamento de tokens.

---

## ☁️ Strata Cloud & Colaboração em Equipe

Para equipes de engenharia que necessitam de:
- **Team Memory**: Memória semântica compartilhada e indexação contínua entre múltiplos desenvolvedores e repositórios.
- **CDC Relay Hub**: Sincronização central em tempo real via PostgreSQL 16 + pgvector e WebSockets.
- **Web Portal & Dashboard**: Visualização gráfica das comunidades arquiteturais, trilha de auditoria e gestão de chaves/permissões (RBAC).

Conheça a plataforma gerenciada em **[Strata Cloud](https://github.com/phfarath/strata-cloud)**.

---

## 📜 Licença

Distribuído sob as licenças **MIT** ou **Apache-2.0**.
Consulte os arquivos [`LICENSE-MIT`](LICENSE-MIT) e [`LICENSE-APACHE`](LICENSE-APACHE) para mais detalhes.

