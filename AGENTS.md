<!-- STRATA_MEMORY_START -->
## Strata Persistent Memory Protocol
- Consult Strata memory tools (`memory_search`, `memory_get`) before planning non-trivial tasks.
- Check known failure anti-patterns before running destructive or complex operations.
- Record durable takeaways via `memory_write`.

### Known Failure Anti-Patterns
- [HIGH] cargo_test execution error: error: package ID specification 'wrong-package-name' did not match any packages
  *Mitigation*: Avoid repeating identical invalid parameters or unverified flags

### Verified Semantic Facts
- Contingency Protocol Omega-7
- Offline-First CDC Engine
- Universal MCP Multi-Version Transport
- Radical Simplicity Principle
- Out-of-Band Silent Error Capture Mechanism
- Strata 3-Point Code Anchoring Architecture
- Tri-Tier Cognitive Memory Hierarchy (Core, Working, Peripheral)
- Tree-Sitter AST Anchoring with Git Merkle Tree
- Bi-Temporal JTMS with Deterministic Truth Maintenance
- Autonomous DPO/KTO/SFT Dataset Mining from Agent Trajectories
- Native Call Graph & Import Dependency Analyzer in Rust (STRATA-T-16)
- Multi-Package Monorepo & Workspace Boundary Isolator (STRATA-T-17)
- Local-First SQLite Persistence with FastEmbed ONNX Vectors
- Cloud Parity Multi-Tenant Backend (PostgreSQL 16, pgvector, Axum) in phfarath/strata-cloud
<!-- STRATA_MEMORY_END -->

## Local Development & Testing Workflow
Strata Open Core runs 100% local-first on the developer's machine:

- **Check compilation across workspace**: `cargo check --workspace`
- **Run all unit & integration tests**: `cargo test --workspace`
- **Run the local MCP Server via Stdio**: `cargo run -p strata-cli --bin strata -- mcp`
- **Run with MCP Inspector**: `npx @modelcontextprotocol/inspector cargo run -p strata-cli --bin strata -- mcp`
- **Launch the interactive Terminal TUI**: `cargo run -p strata-cli --bin strata -- ui`

> Note: The managed multi-tenant cloud backend and Docker stack are maintained separately in [`phfarath/strata-cloud`](https://github.com/phfarath/strata-cloud).

## Engineering Guidelines (Strata)
- **Lean & Clean Code**: Always optimize for minimal necessary code. Avoid premature abstractions, unnecessary boilerplate, or over-engineering.
- **Radical Simplicity**: Prefer direct Rust implementations with well-designed types before adding extra abstraction layers.
- **Atomicity & Modularity**: Each crate must have a strict, well-defined scope with no hidden coupling.
- **Strict TDD**: Every new feature, command, or bug fix must follow the Red -> Green -> Refactor cycle accompanied by unit and integration tests.
