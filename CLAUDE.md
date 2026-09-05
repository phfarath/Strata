<!-- STRATA_MEMORY_START -->
## Strata Persistent Memory & Concurrency Protocol
- Contextual memory and known anti-patterns are automatically injected via hooks on session start and prompt submit.
- Concurrency & Leases: Before modifying any file or crate, acquire an exclusive lease via `lease_acquire(resource_id, ttl_seconds)`. If conflict occurs, yield and select another non-conflicting task. Release via `lease_release(resource_id)` upon completion (mechanical PreTool hooks also block conflicting edits automatically).
- Use the MCP tools (`memory_search`, `memory_get`, `memory_write`, `memory_digest`) when exploring context or persisting verified architectural decisions.
- Execute build/test commands via `strata hook wrap -- <cmd>` to automatically synthesize failure anti-patterns out-of-band.
- Record negative patterns immediately upon encountering dead-ends or tool errors.
<!-- STRATA_MEMORY_END -->


## Local Development & Testing Workflow
Strata Open Core runs 100% local-first on the developer's machine:

- **Check compilation across workspace**: `cargo check --workspace`
- **Run all unit & integration tests**: `cargo test --workspace`
- **Run the local MCP Server via Stdio**: `cargo run -p strata-cli --bin strata -- mcp`
- **Run with MCP Inspector**: `npx @modelcontextprotocol/inspector cargo run -p strata-cli --bin strata -- mcp`

> Note: The managed multi-tenant cloud backend and Docker stack are maintained separately in [`phfarath/strata-cloud`](https://github.com/phfarath/strata-cloud).

## Engineering Guidelines (Strata)
- **Lean & Clean Code**: Always optimize for minimal necessary code. Avoid premature abstractions, unnecessary boilerplate, or over-engineering.
- **Radical Simplicity**: Prefer direct Rust implementations with well-designed types before adding extra abstraction layers.
- **Atomicity & Modularity**: Each crate must have a strict, well-defined scope with no hidden coupling.
- **Strict TDD**: Every new feature, command, or bug fix must follow the Red -> Green -> Refactor cycle accompanied by unit and integration tests.
