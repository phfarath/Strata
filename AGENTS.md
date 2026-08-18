<!-- STRATA_MEMORY_START -->
## Strata Memory Protocol
- Consult Strata memory tools (`memory_search`, `memory_get`) before planning non-trivial tasks.
- Check known failure anti-patterns before running destructive or complex operations.
- Record durable takeaways via `memory_write`.
<!-- STRATA_MEMORY_END -->

## Diretrizes de Engenharia do Projeto (Strata)
- **Código Enxuto e Limpo**: Otimizar sempre para o mínimo de código necessário. Evitar boilerplate, abstrações prematuras ou over-engineering.
- **Simplicidade Radical**: Preferir implementações diretas em Rust com tipos bem desenhados antes de adicionar camadas extras.
- **Atomicidade e Modularidade**: Cada crate deve ter escopo estrito, sem acoplamento oculto.

