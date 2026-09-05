<!-- STRATA_MEMORY_START -->
## Strata Memory & Concurrency Protocol
- Consult Strata memory tools (`memory_search`, `memory_get`) before planning non-trivial tasks.
- Check known failure anti-patterns before running destructive or complex operations.
- Concurrency & Leases: Acquire exclusive leases via `lease_acquire(resource_id, ttl_seconds)` before modifying files or crates to prevent concurrent collisions with other agents (Cursor, Claude Code, Gemini). Release via `lease_release(resource_id)` when finished.
- Wrap test/build commands with `strata hook wrap -- <cmd>` to capture compiler failures out-of-band.
- Record durable takeaways via `memory_write`.
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
