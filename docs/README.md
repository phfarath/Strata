# Cognitive Agent Runtime

Pacote de pesquisa e arquitetura para um agente cognitivo persistente, implementado como runtime em Rust.

## Estrutura

- [01-overview.md](01-overview.md) — problema, escopo e critérios.
- [02-arquitetura-cognitiva.md](02-arquitetura-cognitiva.md) — componentes e ciclo de controle.
- [03-memoria.md](03-memoria.md) — memória episódica, semântica e procedural.
- [04-aprendizado-continuo.md](04-aprendizado-continuo.md) — consolidação, replay e atualização segura.
- [05-world-model.md](05-world-model.md) — crenças, causalidade e previsão.
- [06-reasoning-metacognicao.md](06-reasoning-metacognicao.md) — busca, verificadores e incerteza.
- [07-long-horizon.md](07-long-horizon.md) — objetivos, checkpoints e recuperação.
- [08-embodiment-robotica.md](08-embodiment-robotica.md) — simulador, percepção e ação física.
- [09-runtime-rust.md](09-runtime-rust.md) — desenho do runtime e interfaces.
- [10-roadmap-pesquisa.md](10-roadmap-pesquisa.md) — fases e métricas.
- [11-referencias.md](11-referencias.md) — bibliografia anotada.
- [12-experience-cloud-integracoes.md](12-experience-cloud-integracoes.md) — produto, integrações e eventos canônicos.
- [13-tres-fronteiras-mcp-a2a.md](13-tres-fronteiras-mcp-a2a.md) — MCP hipocampo, memória A2A e metacognição.
- [fluxos-principais.mmd](fluxos-principais.mmd) — diagrama Mermaid editável.

## Princípio de projeto

O LLM é um componente de interpretação e geração, não a fonte única de memória, planejamento, aprendizado ou controle. Essas capacidades são subsistemas persistentes, tipados e auditáveis.
