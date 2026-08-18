<!-- STRATA_MEMORY_START -->
## Strata Persistent Memory Protocol
- Consult Strata memory tools (`memory_search`, `memory_get`) before planning non-trivial tasks.
- Check known failure anti-patterns before running destructive or complex operations.
- Record durable takeaways via `memory_write`.

### Known Failure Anti-Patterns
- [HIGH] cargo_test execution error: error: package ID specification 'wrong-package-name' did not match any packages
  *Mitigation*: Avoid repeating identical invalid parameters or unverified flags

### Verified Semantic Facts
- Offline-First CDC Engine
- Universal MCP Multi-Version
- Radical Simplicity Principle
- Mecanismo Out-of-Band de Captura Silenciosa de Erros
- Arquitetura dos 3 Pontos de Ancoragem do Strata
<!-- STRATA_MEMORY_END -->


## Diretrizes de Engenharia do Projeto (Strata)
- **Código Enxuto e Limpo**: Otimizar sempre para o mínimo de código necessário. Evitar boilerplate, abstrações prematuras ou over-engineering.
- **Simplicidade Radical**: Preferir implementações diretas em Rust com tipos bem desenhados antes de adicionar camadas extras.
- **Atomicidade e Modularidade**: Cada crate deve ter escopo estrito, sem acoplamento oculto.

