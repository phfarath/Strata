# Runtime Rust

## Papel de Rust

Rust é adequado para um runtime persistente: concorrência segura, tipos expressivos e baixo custo operacional. O núcleo confiável mantém plano, estado, ferramentas e auditoria; inferência de LLM pode ser um serviço substituível.

## Módulos

- `core`: tipos de estado, eventos e políticas.
- `memory`: stores, índice vetorial e consolidação.
- `world`: grafo de crenças e previsões.
- `planning`: DAG, scheduler e replanejamento.
- `reasoning`: adaptadores LLM, busca e verificadores.
- `tools`: schemas, permissões, executores e sandbox.
- `evals`: cenários, métricas e regressões.
- `api`: CLI/HTTP e observabilidade.

## Interfaces essenciais

Definir traits para `MemoryStore.retrieve`, `Tool.invoke`, `Verifier.verify` e `Planner.next`. Toda implementação concreta deve poder ser substituída por uma versão local, remota ou simulada.

## Eventos e persistência

Use event sourcing: `ObservationReceived`, `PlanCreated`, `ActionAuthorized`, `ToolInvoked`, `OutcomeObserved` e `MemoryConsolidated`. Materializações podem ser reconstruídas; eventos dão auditoria e replay experimental.

## Dependências prováveis

Tokio, Serde, SQLx, Axum, tracing, UUID, time e uma camada de filas. Mantenha vetores, grafo e LLM atrás de traits.
