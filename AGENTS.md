<!-- STRATA_MEMORY_START -->
## Strata Memory & Concurrency Protocol
- Consult Strata memory tools (`memory_search`, `memory_get`) before planning non-trivial tasks.
- Check known failure anti-patterns before running destructive or complex operations.
- Concurrency & Leases: Acquire exclusive leases via `lease_acquire(resource_id, ttl_seconds)` or `strata lease acquire <resource_id>` before modifying files or crates to prevent concurrent collisions with other agents (Cursor, Claude Code, Gemini). Release via `lease_release(resource_id)` or `strata lease release <resource_id>` when finished.
- Realtime Event Stream: Monitor concurrent peer agent leases and anti-patterns with `strata a2a listen --json`.
- Wrap test/build commands with `strata hook wrap -- <cmd>` to capture compiler failures out-of-band.
- Record durable takeaways via `memory_write`.

## Multi-Agent CLI Blackboard & A2A Collaboration
Agents running concurrently on the local machine (Claude Code, Cursor Agent, Gemini CLI, scripts) collaborate without centralized orchestrator daemons using the SQLite WAL blackboard and IPC bus:

### 1. Agent Environment Identity
Set these environment variables in your agent subshell or let Strata auto-detect them:
- `STRATA_AGENT_ID`: Unique agent identity (e.g. `claude-worker-1`, `cursor-agent`).
- `STRATA_AGENT_HOST`: Agent runner (e.g. `claude-code`, `cursor`, `gemini-cli`, `antigravity`).
- `STRATA_TASK_ID`: Currently assigned task ID.

### 2. Goal Planning & Task Decomposition
Decompose high-level goals into dependency DAGs persisted directly to the shared blackboard:
```bash
strata plan "Implement auth token refresh" --persist
```

### 3. Atomic Task Work-Stealing & Execution
Claim ready tasks atomically without race conditions, post progress updates, and mark completion with artifacts:
```bash
# Claim next available task whose dependencies are satisfied
strata task claim --next

# Update progress
strata task progress <task-id> --note "Wrote unit test"

# Complete task (automatically releases associated leases and unblocks downstream tasks)
strata task complete <task-id> --summary "Implemented and verified" --artifact src/auth.rs

# Or report failure
strata task fail <task-id> --error "Missing dependency"

# Inspect task status and artifacts
strata task inspect <task-id>
strata task list --status ready
```

### 4. Direct Agent-to-Agent (A2A) Messaging & Synchronization
Communicate directly with peer agents or synchronize on asynchronous events:
```bash
# Send direct message to a peer agent
strata a2a send <target-agent-id> "Can you review src/auth.rs?"

# Broadcast coordination message to all active agents
strata a2a broadcast "Refactoring core crate, please release leases"

# Read mailbox
strata a2a inbox --unread

# Wait synchronously for event or peer response
strata a2a wait --timeout 30 --event direct_message
```
<!-- STRATA_MEMORY_END -->


## Local Development & Testing Workflow
Strata Open Core runs 100% local-first on the developer's machine:

- **Check compilation across workspace**: `cargo check --workspace`
- **Run all unit & integration tests**: `cargo test --workspace`
- **Listen to real-time A2A IPC event bus**: `cargo run -p strata-cli --bin strata -- a2a listen`
- **Run the local MCP Server via Stdio**: `cargo run -p strata-cli --bin strata -- mcp`
- **Run with MCP Inspector**: `npx @modelcontextprotocol/inspector cargo run -p strata-cli --bin strata -- mcp`
- **Launch the interactive Terminal TUI**: `cargo run -p strata-cli --bin strata -- ui`

> Note: The managed multi-tenant cloud backend and Docker stack are maintained separately in [`phfarath/strata-cloud`](https://github.com/phfarath/strata-cloud).

## Engineering Guidelines (Strata)
- **Lean & Clean Code**: Always optimize for minimal necessary code. Avoid premature abstractions, unnecessary boilerplate, or over-engineering.
- **Radical Simplicity**: Prefer direct Rust implementations with well-designed types before adding extra abstraction layers.
- **Atomicity & Modularity**: Each crate must have a strict, well-defined scope with no hidden coupling.
- **Strict TDD**: Every new feature, command, or bug fix must follow the Red -> Green -> Refactor cycle accompanied by unit and integration tests.
