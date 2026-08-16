# Experience Cloud — Integrações e Produto

## Tese do produto

O produto não é uma API de memória isolada. É uma camada organizacional de experiência compartilhada: conectar uma vez e permitir que os agentes autorizados de uma equipe aprendam com resultados produzidos por outros agentes.

> Connect once. Every agent your team uses learns from every other agent.

O usuário não precisa conhecer MCP, hooks, SDKs ou chaves de API. A experiência externa deve ser limitada a escolher um cliente, autenticar e autorizar.

```text
Escolher cliente → Autorizar Experience → Conectado
```

## Arquitetura de integração

Existem duas interfaces complementares, independentes dos clientes.

| Interface | Direção | Função |
|---|---|---|
| Read side: MCP | agente → Experience | recuperar memórias, habilidades, padrões e alertas |
| Write side: Event Ingestion | agente → Experience | registrar tarefas, ações, resultados e falhas |

Todo cliente externo é um adaptador. O núcleo não depende de Claude, Cursor, Codex, ChatGPT, Gemini ou Windsurf.

```text
Clientes e agentes
  ├─ MCP ────────────────► Experience API: leitura
  └─ hooks / OTEL / SDK ─► Event API: escrita
                                  │
                                  ▼
                         Experience Engine
                         ├─ Memory Engine
                         ├─ Experience Engine
                         └─ Skills Engine
```

## Esquema canônico de eventos

Os adaptadores convertem eventos específicos do fornecedor para um formato único. O armazenamento e a aprendizagem não devem conter lógica dependente do cliente de origem.

| Evento | Campos mínimos |
|---|---|
| `SessionStarted` | organização, agente, sessão, ambiente |
| `GoalCreated` | objetivo, escopo, risco, orçamento |
| `PlanCreated` | plano, dependências, versão |
| `ToolStarted` / `ToolCompleted` | ferramenta, entrada resumida, resultado, duração |
| `FileChanged` | repositório, caminho, diffs ou referência segura |
| `CommandExecuted` / `TestExecuted` | comando, status, artefatos, duração |
| `ErrorObserved` | classificação, evidência, impacto |
| `TaskCompleted` | critério de sucesso, resultado, avaliação |
| `SessionEnded` | estado final, checkpoint, consolidação pendente |

Eventos devem portar `event_id`, instante, organização, sessão, agente, proveniência, classificação de dados e política de retenção. Segredos e conteúdo sensível não podem entrar na telemetria bruta.

## Superfície MCP inicial

- `search_experience`: retorna episódios e procedimentos relevantes, com fonte e confiança.
- `record_outcome`: registra resultado validado de uma tarefa.
- `get_known_failures`: retorna falhas recorrentes e suas evidências.
- `get_memory_capabilities`: descreve escopo, retenção e controles de acesso.

O gateway aplica isolamento por organização, repositório, projeto e usuário. Recuperação sem ACL explícita é proibida.

## Prioridade de integrações

| Prioridade | Integração | Papel |
|---|---|---|
| 1 | Claude Code | MCP para leitura e plugin/hooks para ciclos de execução |
| 1 | Cursor | instalação simples de MCP e autenticação |
| 1 | Codex | MCP para leitura; eventos por mecanismos oficialmente suportados |
| 2 | ChatGPT | ferramentas remotas autorizadas, quando a superfície MCP aplicável estiver disponível |
| 2 | Gemini CLI | extensão/MCP e eventos suportados pelo cliente |
| 3 | Windsurf e outros | adaptadores submetidos ao mesmo contrato canônico |

Cada alegação de suporte específico por cliente deve ser validada contra sua documentação oficial na implementação. O runtime não deve pressupor telemetria, hooks ou permissões inexistentes.

## MVP

Escopo restrito: Claude Code, Cursor e Codex; MCP remoto, OAuth, ingestão de eventos, armazenamento episódico e três ferramentas MCP. A hipótese mensurável é: uma experiência validada, gerada por um cliente, reduz a taxa de uma falha equivalente em outro cliente.

## Métricas

- Redução de falhas repetidas entre clientes.
- Precisão e utilidade das experiências recuperadas.
- Latência de recuperação e ingestão.
- Taxa de eventos descartados por política de privacidade.
- Cobertura de proveniência e ACL.
