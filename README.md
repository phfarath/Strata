# Cognitive Agent Runtime (mem-research)

Runtime cognitivo persistente e camada de memória compartilhada para agentes autônomos.

## Princípio de Projeto

O LLM é um componente de interpretação e geração, não a fonte única de memória, planejamento, aprendizado ou controle. As capacidades cognitivas (memória de trabalho/episódica/semântica/procedural, world model, planejamento hierárquico, verificação e aprendizado contínuo) são subsistemas persistentes, tipados e auditáveis desenvolvidos em Rust.

```text
Clientes e Agentes (Claude Code, Cursor, Codex, Gemini)
       │
       ├─ Leitura (MCP) ────────► Memory & Experience Engine
       └─ Escrita (Eventos) ────► Ingestion & Consolidação
                                        │
                                        ▼
                             Cognitive Agent Runtime (Rust)
                             ├─ Core (Estado & Event Sourcing)
                             ├─ Memory (Episódica, Semântica, Procedural)
                             ├─ World Model (Grafo de Crenças)
                             ├─ Planning (DAG de Subobjetivos)
                             ├─ Reasoning & Verifier (Metacognição)
                             └─ Tool Gateway (Permissões & Sandbox)
```

---

## Índice da Documentação

A especificação técnica e de pesquisa está organizada em [`docs/`](docs/):

### 1. Fundamentos e Arquitetura
- [01. Overview](docs/01-overview.md) — Problema, escopo, critérios e hipótese central.
- [02. Arquitetura Cognitiva](docs/02-arquitetura-cognitiva.md) — Componentes, ciclo de controle (observar–decidir–agir) e invariantes.
- [03. Memória](docs/03-memoria.md) — Memória de trabalho, episódica, semântica e procedural; recuperação híbrida e consolidação.
- [04. Aprendizado Contínuo](docs/04-aprendizado-continuo.md) — Consolidação não-paramétrica, extração de habilidades, replay curado e proteções contra esquecimento catastrófico.

### 2. Cognição e Execução
- [05. World Model](docs/05-world-model.md) — Grafo de crenças, relações causais, erro de previsão e planejamento contrafactual.
- [06. Reasoning e Metacognição](docs/06-reasoning-metacognicao.md) — Busca guiada, verificadores determinísticos, calibração e regra de decisão.
- [07. Long-Horizon Autonomy](docs/07-long-horizon.md) — DAG de objetivos, checkpoints, idempotência, governança de risco e recuperação.
- [08. Embodiment e Robótica](docs/08-embodiment-robotica.md) — Ciclo fechado percepção–ação, controlador reativo vs. deliberativo.

### 3. Implementação e Engenharia
- [09. Runtime Rust](docs/09-runtime-rust.md) — Desenho de módulos (`core`, `memory`, `world`, `planning`, `reasoning`, `tools`, `evals`, `api`), traits e event sourcing.
- [10. Roadmap de Pesquisa](docs/10-roadmap-pesquisa.md) — Fases de execução (Fase 0 a Fase 5), métricas e disciplina experimental.
- [11. Referências](docs/11-referencias.md) — Bibliografia anotada de papers seminais e literatura de sistemas.

### 4. Produto e Protocolos
- [12. Experience Cloud & Integrações](docs/12-experience-cloud-integracoes.md) — Tese de memória compartilhada inter-agentes, esquema canônico de eventos e superfície MCP inicial.
- [13. Três Fronteiras: MCP e A2A](docs/13-tres-fronteiras-mcp-a2a.md) — Hipocampo externo MCP, memória transativa A2A e governança metacognitiva.

### 5. Diagramas
- [Fluxos Principais (Mermaid)](docs/diagrams/fluxos-principais.mmd) — Diagrama de fluxo de controle, memória e consolidação.

---

## Estrutura do Repositório

```text
mem-research/
├── docs/                      # Especificações arquiteturais e de pesquisa
│   ├── 01-overview.md ... 13-tres-fronteiras-mcp-a2a.md
│   ├── diagrams/              # Diagramas de arquitetura (Mermaid)
│   └── README.md
├── crates/                    # Módulos do runtime em Rust (a iniciar)
│   ├── core/                  # Tipos de estado, eventos canônicos e contratos
│   ├── memory/                # Stores, indexação vetorial e consolidação
│   ├── world/                 # Grafo de crenças e predição causal
│   ├── planning/              # DAG de metas e scheduler
│   ├── reasoning/             # Adaptadores LLM, busca e verificadores
│   ├── tools/                 # Gateway de ferramentas, permissões e sandbox
│   ├── evals/                 # Cenários determinísticos, benchmarks e regressões
│   └── api/                   # MCP Server, REST/HTTP e observabilidade
└── README.md
```
