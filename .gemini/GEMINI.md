<!-- STRATA_MEMORY_START -->
## Strata Memory & Concurrency Protocol
- Check Strata memory pointers and known anti-patterns before complex tasks.
- Concurrency & Leases: Call `lease_acquire(resource_id, ttl_seconds)` before editing files/crates. If conflict is reported, select another non-conflicting module.
- Wrap build/test runs with `strata hook wrap -- <cmd>` to automatically learn failure guardrails.
- Persist verified solutions and architectural guidelines with `memory_write`.
<!-- STRATA_MEMORY_END -->



