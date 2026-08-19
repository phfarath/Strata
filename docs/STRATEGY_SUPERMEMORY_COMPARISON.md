# Strata vs Supermemory: Tese Estratégica & Arquitetura de Diferenciação

> **Supermemory** = *"Give your AI perfect recall."*  
> **Strata** = *"Give agents a persistent cognitive state & continual learning."*

---

## 1. Visão Geral e Posicionamento

O Supermemory posiciona-se como **infraestrutura de contexto e recall** para IA: extração automática de preferências/fatos, perfis de usuário, grafos temporais, RAG, conectores (Google Drive, Gmail, Notion, GitHub), MCP e plugins.

A tese do **Strata** não é ser "um Supermemory reescrito em Rust", mas sim construir uma **camada de inteligência e cognição persistente** onde agentes de software acumulam experiência real ao longo do tempo.

```text
                    STRATA COGNITIVE RUNTIME

 Events / Conversations / Actions / Tools / File Changes
                         │
                         ▼
                  ┌──────────────┐
                  │ Observation  │
                  │   Engine     │
                  └──────┬───────┘
                         │
                         ▼
              ┌─────────────────────┐
              │ Memory Classifier   │
              └─────────┬───────────┘
                        │
       ┌────────────────┼──────────────────┐
       ▼                ▼                  ▼
   Episodic         Semantic          Procedural
   Memory           Memory            Memory
   (Events/Logs)    (JTMS Facts)      (Learned Skills)
       │                │                  │
       └────────────────┼──────────────────┘
                        ▼
              ┌──────────────────┐
              │ Consolidation    │
              │ + Contradiction  │
              │ + Forgetting     │
              └────────┬─────────┘
                       ▼
            Truth Maintenance Graph (JTMS)
                       │
                       ▼
            Context Selection Engine
                       │
             token budget / intent / causality
                       │
                       ▼
               CONNECTED AGENTS
          (Cursor, Claude Code, Codex, Gemini)
```

---

## 2. Pilares Fundamentais de Diferenciação

### 1. Procedural Memory (Agent Learning)
Um agente cognitivo não deve apenas lembrar que *"o usuário prefere Rust"*, mas sim lembrar de estratégias de resolução de problemas:
> *"Na última vez que fizemos deploy deste serviço no Railway, falhou porque faltava a porta dinâmica. A sequência funcional é A → B → C."*

```yaml
procedure:
  task: deploy_backend_railway
  learned_strategy:
    1. build multi-stage Dockerfile
    2. remove static VOLUME directive
    3. track Cargo.lock in git
    4. bind to dynamic PORT env var
  confidence: 0.96
  learned_from:
    - execution_812
    - execution_940
  failure_patterns_mitigated:
    - docker_volume_unsupported
```

### 2. Memória com Proveniência Causal (Why Believe X?)
Toda crença no Strata é rastreável e auditável:
```text
Memory Record:
├── id
├── content
├── type (Episodic | Semantic | Procedural | NegativePattern)
├── source (User | Agent | Tool | Linter)
├── timestamp
├── confidence (0.0 to 1.0)
├── evidence[] (commits, files, execution logs)
├── derived_from[]
├── contradictions[]
├── supersedes[] (JTMS belief revision)
└── usefulness_score
```

Quando o agente toma uma decisão, ele consegue justificar:
> *"Eu acredito em X (confiança: 0.92) com base na conversa #182, commit `87ac31` e execução #922."*

### 3. Memory ≠ Context (Context Selection Engine)
O banco de dados pode conter 100.000 memórias, mas o prompt do LLM aceita apenas uma fração restrita sem poluir o raciocínio.
O motor de seleção de contexto filtra as ~8 memórias ideais baseando-se em:
- Relevância Causal & de Tarefa
- Recência & Curva de Decaimento ACT-R / Ebbinghaus
- Confiança & Não-Contradição
- Restrição estrita de Token Budget (~300-500 tokens por digest)

### 4. Rust Memory Runtime (Ultra-Portable & Low Overhead)
Rust não é a proposta de valor por si só, mas viabiliza um **Runtime Cognitivo Embutido**:
- In-process SQLite WAL local com zero latência (< 5ms).
- Daemon local leve (< 10MB de RAM).
- Binário único multiplataforma (`strata`).
- Protocolo universal MCP (JSON-RPC) para agentes sem dependências externas pesadas.

---

## 3. Matriz Competitiva

| Capacidade | Supermemory | Strata (Proposta Estratégica) |
|---|---|---|
| RAG & Embeddings | ✅ | ✅ |
| Semantic Memory | ✅ | ✅ |
| Episodic Memory | ✅ | ✅ |
| Temporal Graph | ✅ | ✅ (com JTMS Belief Revision) |
| Mathematical Forgetting | ✅ | ✅ (ACT-R + Ebbinghaus) |
| User Profile | ✅ | ✅ |
| Conectores Externos | ✅ (Google Drive, Notion, etc.) | Foco inicial em repositórios & código |
| MCP Support | ✅ | ✅ (Multi-versão universal) |
| Shared Memory Multi-Agent | ✅ | ✅ (Cloud Sync CDC + SQLite WAL) |
| **Procedural Learning (Skills)** | Secundário / Feature | **Pilar Central (Core)** |
| **Memory Provenance & Justification** | Limitado | **Pilar Central (Core)** |
| **Failure Pattern Auto-Capture** | Oportunidade | **Pilar Central (Core)** |
| **Continual Agent Learning** | Oportunidade | **Produto Central** |
| **Embedded Rust Native Runtime** | Node/Py | **Rust Native Core (< 10MB RAM)** |

---

## 4. O Wedge Inicial: Coding Agents
Em vez de tentar competir em 20 integrações genéricas de SaaS imediatamente, o Strata foca obsessivamente em **Coding Agents**:

```text
                  STRATA RUNTIME
                        │
       ┌────────────────┼────────────────┐
       ▼                ▼                ▼
  Claude Code         Cursor           Codex / Gemini
       │                │                │
       └────────────────┼────────────────┘
                        ▼
               Mesma Memória Compartilhada
```

Com um simples `strata init` ou `strata login`, qualquer agente operando no repositório ganha acesso imediato a todas as decisões, heurísticas, procedimentos e correções passadas da equipe.

---

## 5. Primitivas Centrais da Engine

1. `remember(event)`: Captura de eventos, decisões e interações.
2. `recall(context, budget)`: Recuperação contextual inteligente respeitando o limite de tokens.
3. `learn(outcome)`: Destilação automática de sucessos/falhas em procedimentos e padrões negativos.
4. `forget(threshold)`: Limpeza de memórias antigas com baixa utilidade e decaimento temporal.
5. `explain(memory_id)`: Explicação completa da cadeia de evidências e crenças (JTMS).
