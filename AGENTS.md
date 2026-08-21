<!-- STRATA_MEMORY_START -->
## Strata Persistent Memory Protocol
- Consult Strata memory tools (`memory_search`, `memory_get`) before planning non-trivial tasks.
- Check known failure anti-patterns before running destructive or complex operations.
- Record durable takeaways via `memory_write`.

### Known Failure Anti-Patterns
- [HIGH] cargo_test execution error: error: package ID specification 'wrong-package-name' did not match any packages
  *Mitigation*: Avoid repeating identical invalid parameters or unverified flags

### Verified Semantic Facts
- Protocolo de Contingência Ômega-7
- Offline-First CDC Engine
- Universal MCP Multi-Version
- Radical Simplicity Principle
- Mecanismo Out-of-Band de Captura Silenciosa de Erros
- Arquitetura dos 3 Pontos de Ancoragem do Strata
- Arquitetura Tri-Tier Cognitiva (Core, Working, Peripheral)
- Ancoragem AST Tree-Sitter com Merkle Tree Git
- JTMS Bi-Temporal com Truth Maintenance Determinístico
- Mineração Autônoma de Datasets DPO/KTO a partir de Trajetórias de Código
- Native Call Graph & Import Dependency Analyzer em Rust (STRATA-T-16)
- Multi-Package Monorepo & Workspace Boundaries Isolator (STRATA-T-17)
- Ambiente Local Docker com Paridade 100% Cloud (PostgreSQL 16, pgvector, Axum Server)
<!-- STRATA_MEMORY_END -->

## Ambiente Local no Docker (Paridade com Nuvem)
O repositório possui uma stack Docker Compose pronta que replica exatamente o ambiente de nuvem (Railway + Supabase/Neon PostgreSQL com extensão `pgvector`):

- **Subir a stack completa**: `docker compose up -d`
- **Verificar saúde e status**: `curl http://localhost:8080/health` (ou `docker ps`)
- **Executar suíte de testes E2E local**: `powershell .\docker\test_local_stack.ps1`
- **Derrubar a stack**: `docker compose down`

### Serviços Docker:
1. `strata-db`: PostgreSQL 16 com extensão `pgvector` e `uuid-ossp` na porta `5432`.
2. `strata-server`: Servidor Rust Axum com CDC sync, SaaS auth, pgvector embeddings e WebSockets na porta `8080`.

## Diretrizes de Engenharia do Projeto (Strata)
- **Código Enxuto e Limpo**: Otimizar sempre para o mínimo de código necessário. Evitar boilerplate, abstrações prematuras ou over-engineering.
- **Simplicidade Radical**: Preferir implementações diretas em Rust com tipos bem desenhados antes de adicionar camadas extras.
- **Atomicidade e Modularidade**: Cada crate deve ter escopo estrito, sem acoplamento oculto.
- **TDD Rigoroso**: Todo novo recurso ou comando deve ser construído via ciclo Red -> Green -> Refactor acompanhado de testes unitários e E2E.
