use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Instant;
use uuid::Uuid;

use strata_core::schemas::{EpisodeRecord, EpisodeStep};
use strata_memory::{
    EpisodicPyramidEngine, EpisodicPyramidStore, PyramidConfig, PyramidFilter, SqliteStore,
};

/// Evaluation scenario metrics report for Hierarchical Episodic Memory Pyramid.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HierarchicalPyramidEvalReport {
    pub raw_tier1_tokens: usize,
    pub rollup_tier2_tokens: usize,
    pub compression_ratio: f64,
    pub drill_down_accuracy: f64,
    pub is_deterministic: bool,
    pub clusters_count: usize,
    pub total_episodes: usize,
    pub total_steps: usize,
    pub latency_ms: f64,
}

/// Helper function to estimate token count from an EpisodeStep.
/// Uses standard ~4 characters per token heuristic over all fields.
fn estimate_step_tokens(step: &EpisodeStep) -> usize {
    let mut chars = step.action.len();
    if let Some(ref t) = step.tool {
        chars += t.len();
    }
    if let Some(ref inp) = step.input {
        chars += serde_json::to_string(inp).map(|s| s.len()).unwrap_or(0);
    }
    if let Some(ref out) = step.output {
        chars += serde_json::to_string(out).map(|s| s.len()).unwrap_or(0);
    }
    if let Some(ref err) = step.error {
        chars += err.len();
    }
    (chars + 3) / 4
}

/// Helper function to estimate token count from a Tier 2 Rollup EpisodeRecord.
fn estimate_rollup_tokens(rollup: &EpisodeRecord) -> usize {
    let mut chars = rollup.summary.len();
    for out in &rollup.outcomes {
        chars += out.len();
    }
    if let Some(ref prov) = rollup.causal_provenance {
        chars += prov.len();
    }
    (chars + 3) / 4
}

fn deterministic_uuid(prefix: u128, index: usize) -> Uuid {
    Uuid::from_u128((prefix << 64) | (index as u128 + 1))
}

type StepDef = (
    &'static str,
    &'static str,
    serde_json::Value,
    serde_json::Value,
    Option<&'static str>,
);

type EpisodeDef = (
    &'static str,
    f32,
    Vec<&'static str>,
    Vec<StepDef>,
    Option<&'static str>,
);

/// Builds 18 realistic synthetic episodes distributed across 3 distinct clusters:
/// 1. "Auth JWT Migration" (task_id: "task-auth-jwt", 6 episodes)
/// 2. "Database SQLite Pooling" (task_id: "task-sqlite-pool", 6 episodes, contains failure step)
/// 3. "CDC Outbox Worker" (task_id: "task-cdc-outbox", 6 episodes)
pub fn build_synthetic_pyramid_episodes(session_id: &str) -> (Vec<EpisodeRecord>, Uuid, u32) {
    let base_time: i64 = 1700000000;
    let mut episodes = Vec::with_capacity(18);

    // Track expected failure target (SQLite episode index 3)
    let target_failure_ep_id = deterministic_uuid(0x2000, 3);
    let target_failure_step_index = 1u32;

    // -------------------------------------------------------------------------
    // Cluster 1: Auth JWT Migration (6 episodes)
    // -------------------------------------------------------------------------
    let auth_defs: Vec<EpisodeDef> = vec![
        (
            "Generate RSA-256 Keypair for JWT Signing",
            0.85f32,
            vec![
                "RSA keypair generated in PEM format",
                "Keys securely stored in vault",
            ],
            vec![
                (
                    "Execute openssl genrsa to generate private key in src/auth/jwt_private.pem",
                    "run_command",
                    serde_json::json!({ "command": "openssl genrsa -out src/auth/jwt_private.pem 2048", "cwd": "/repo" }),
                    serde_json::json!({
                        "stdout": "Generating RSA private key, 2048 bit long modulus (2 primes)\n................+++++\n....................................................................+++++\ne is 65537 (0x010001)\nwriting RSA key to 'src/auth/jwt_private.pem'",
                        "exit_code": 0,
                        "file_path": "src/auth/jwt_private.pem"
                    }),
                    None,
                ),
                (
                    "Extract public key to src/auth/jwt_public.pem",
                    "run_command",
                    serde_json::json!({ "command": "openssl rsa -in src/auth/jwt_private.pem -pubout -out src/auth/jwt_public.pem" }),
                    serde_json::json!({
                        "stdout": "writing RSA key\nPublic key written to src/auth/jwt_public.pem (451 bytes)",
                        "exit_code": 0,
                        "file_path": "src/auth/jwt_public.pem"
                    }),
                    None,
                ),
                (
                    "Verify key integrity with openssl rsa -check",
                    "run_command",
                    serde_json::json!({ "command": "openssl rsa -in src/auth/jwt_private.pem -check" }),
                    serde_json::json!({
                        "stdout": "RSA key ok\nModulus length: 2048 bits\nExponent: 65537\nPrime factor validation completed successfully.",
                        "exit_code": 0
                    }),
                    None,
                ),
            ],
            Some("Auth team RFC-042 JWT migration"),
        ),
        (
            "Implement Token Minting and Claims Validation",
            0.90f32,
            vec![
                "JWT claim structure standardized",
                "Issuer and audience validated",
            ],
            vec![
                (
                    "Define Claims struct in src/auth/claims.rs with sub, exp, iss, aud",
                    "replace_file_content",
                    serde_json::json!({ "file_path": "src/auth/claims.rs", "fields": ["sub", "exp", "iss", "aud", "roles"] }),
                    serde_json::json!({
                        "status": "success",
                        "file_path": "src/auth/claims.rs",
                        "diff": "@@ -1,5 +1,18 @@\n+#[derive(Debug, Serialize, Deserialize)]\n+pub struct Claims {\n+    pub sub: String,\n+    pub exp: i64,\n+    pub iss: String,\n+    pub aud: String,\n+    pub roles: Vec<String>,\n+}"
                    }),
                    None,
                ),
                (
                    "Implement sign_token in src/auth/token.rs using RS256 algorithm",
                    "replace_file_content",
                    serde_json::json!({ "file_path": "src/auth/token.rs", "algorithm": "RS256", "ttl_secs": 3600 }),
                    serde_json::json!({
                        "status": "success",
                        "file_path": "src/auth/token.rs",
                        "signature_mode": "Asymmetric RS256 with PKCS#1 v1.5 padding",
                        "cache_policy": "In-memory signing key cache with 30s reload interval"
                    }),
                    None,
                ),
                (
                    "Run cargo test over auth token module",
                    "run_command",
                    serde_json::json!({ "command": "cargo test -p strata-auth --lib token_tests" }),
                    serde_json::json!({
                        "stdout": "running 6 tests\ntest tests::test_token_minting_rs256 ... ok\ntest tests::test_claims_expiration_verification ... ok\ntest tests::test_issuer_mismatch_rejection ... ok\ntest tests::test_audience_validation ... ok\ntest tests::test_malformed_signature_failure ... ok\ntest tests::test_tampered_payload_detection ... ok\n\ntest result: ok. 6 passed; 0 failed",
                        "exit_code": 0
                    }),
                    None,
                ),
            ],
            Some("Auth team RFC-042 JWT migration"),
        ),
        (
            "Add JWKS Endpoint for Asymmetric Key Rotation",
            0.80f32,
            vec![
                "JWKS endpoint exposed at /.well-known/jwks.json",
                "Key ID kid added to header",
            ],
            vec![
                (
                    "Create JWKS response serializer in src/auth/jwks.rs",
                    "write_to_file",
                    serde_json::json!({ "file_path": "src/auth/jwks.rs", "keys": 2 }),
                    serde_json::json!({
                        "file_path": "src/auth/jwks.rs",
                        "jwks_output": {
                            "keys": [
                                { "kty": "RSA", "use": "sig", "alg": "RS256", "kid": "key-2026-v1", "n": "u1...==", "e": "AQAB" },
                                { "kty": "RSA", "use": "sig", "alg": "RS256", "kid": "key-2026-v2", "n": "w9...==", "e": "AQAB" }
                            ]
                        }
                    }),
                    None,
                ),
                (
                    "Register route /.well-known/jwks.json in src/api/routes.rs",
                    "replace_file_content",
                    serde_json::json!({ "file_path": "src/api/routes.rs", "route": "/.well-known/jwks.json" }),
                    serde_json::json!({
                        "status": "success",
                        "file_path": "src/api/routes.rs",
                        "handler": "get(handle_jwks_discovery)",
                        "cache_headers": "public, max-age=3600, stale-while-revalidate=86400"
                    }),
                    None,
                ),
            ],
            Some("Auth team RFC-042 JWT migration"),
        ),
        (
            "Deprecate Legacy HS256 HMAC Secret Verification",
            0.75f32,
            vec!["HS256 fallback removed", "All clients migrating to RS256"],
            vec![
                (
                    "Scan src/auth/verifier.rs for HS256 algorithm references",
                    "run_command",
                    serde_json::json!({ "command": "rg 'HS256' src/auth/verifier.rs" }),
                    serde_json::json!({
                        "stdout": "src/auth/verifier.rs:42:  Algorithm::HS256 => verify_hmac(token, secret)?,\nsrc/auth/verifier.rs:89:  warn!(\"Legacy HS256 verification requested for {}\", kid);\nsrc/auth/verifier.rs:114: const ALLOW_LEGACY_HS256: bool = true;",
                        "occurrences": 3,
                        "file_path": "src/auth/verifier.rs"
                    }),
                    None,
                ),
                (
                    "Remove HS256 secret verification in src/auth/verifier.rs",
                    "replace_file_content",
                    serde_json::json!({ "file_path": "src/auth/verifier.rs", "diff": "-50 lines" }),
                    serde_json::json!({
                        "status": "success",
                        "file_path": "src/auth/verifier.rs",
                        "removed_symbols": ["verify_hmac", "ALLOW_LEGACY_HS256", "hmac_secret_key"]
                    }),
                    None,
                ),
                (
                    "Run auth integration test suite against test realm",
                    "run_command",
                    serde_json::json!({ "command": "cargo test -p strata-auth --test integration_auth" }),
                    serde_json::json!({
                        "stdout": "running 14 tests\ntest integration::test_rs256_full_login_flow ... ok\ntest integration::test_jwks_remote_key_fetching ... ok\ntest integration::test_rejected_hs256_header ... ok\ntest integration::test_tampered_token_401 ... ok\n\ntest result: ok. 14 passed; 0 failed",
                        "exit_code": 0
                    }),
                    None,
                ),
            ],
            Some("Auth team RFC-042 JWT migration"),
        ),
        (
            "Migrate Refresh Token Revocation Storage to Redis",
            0.85f32,
            vec![
                "Redis cluster store connected for blacklist",
                "Instant token revocation operational",
            ],
            vec![
                (
                    "Implement Redis connection pool in src/storage/redis.rs",
                    "write_to_file",
                    serde_json::json!({ "file_path": "src/storage/redis.rs", "pool_size": 10 }),
                    serde_json::json!({
                        "file_path": "src/storage/redis.rs",
                        "driver": "redis-rs / deadpool-redis",
                        "retry_policy": "Exponential backoff max 3 attempts",
                        "ping_interval_secs": 15
                    }),
                    None,
                ),
                (
                    "Write revocation check in src/auth/revocation.rs with 7-day TTL",
                    "replace_file_content",
                    serde_json::json!({ "file_path": "src/auth/revocation.rs", "ttl_days": 7 }),
                    serde_json::json!({
                        "status": "success",
                        "file_path": "src/auth/revocation.rs",
                        "method": "is_token_revoked(jti: &Uuid) -> bool",
                        "redis_key_pattern": "strata:auth:revoked:{jti}"
                    }),
                    None,
                ),
            ],
            Some("Auth team RFC-042 JWT migration"),
        ),
        (
            "Add Middleware for Bearer Token Extraction and Rate Limiting",
            0.80f32,
            vec![
                "Axum auth middleware attached",
                "Rate limit 100 req/s per user",
            ],
            vec![
                (
                    "Create BearerAuth layer in src/middleware/auth.rs",
                    "write_to_file",
                    serde_json::json!({ "file_path": "src/middleware/auth.rs", "layer": "tower" }),
                    serde_json::json!({
                        "file_path": "src/middleware/auth.rs",
                        "struct": "BearerAuthLayer<S>",
                        "features": ["Authorization header parsing", "Bearer prefix strip", "Claims extraction to Extension<Claims>"]
                    }),
                    None,
                ),
                (
                    "Attach token bucket rate limiter in src/middleware/rate_limit.rs",
                    "replace_file_content",
                    serde_json::json!({ "file_path": "src/middleware/rate_limit.rs", "rate": 100 }),
                    serde_json::json!({
                        "file_path": "src/middleware/rate_limit.rs",
                        "bucket_capacity": 100,
                        "refill_rate_per_sec": 10,
                        "rejection_response": "HTTP 429 Too Many Requests with Retry-After header"
                    }),
                    None,
                ),
            ],
            Some("Auth team RFC-042 JWT migration"),
        ),
    ];

    for (i, (summary, salience, outcomes, steps, prov)) in auth_defs.into_iter().enumerate() {
        let ep_id = deterministic_uuid(0x1000, i);
        let mut ep_steps = Vec::new();
        for (s_idx, (action, tool, inp_val, out_val, err_opt)) in steps.into_iter().enumerate() {
            let mut step = EpisodeStep::new(s_idx as u32, action)
                .with_tool(tool)
                .with_input(inp_val)
                .with_output(out_val);
            if let Some(err) = err_opt {
                step = step.with_error(err);
            }
            ep_steps.push(step);
        }
        let outcomes_vec: Vec<String> = outcomes.into_iter().map(String::from).collect();
        let mut ep = EpisodeRecord::new(session_id, "agent-auth-lead", summary)
            .with_salience(salience)
            .with_task_id("task-auth-jwt")
            .with_steps(ep_steps)
            .with_outcomes(outcomes_vec);
        ep.id = ep_id;
        ep.created_at = base_time + (i as i64 * 60);
        if let Some(p) = prov {
            ep = ep.with_provenance(p);
        }
        episodes.push(ep);
    }

    // -------------------------------------------------------------------------
    // Cluster 2: Database SQLite Pooling (6 episodes, ep 3 has failure)
    // -------------------------------------------------------------------------
    let sqlite_defs: Vec<EpisodeDef> = vec![
        (
            "Configure SQLite WAL Mode and Busy Timeout Pragma",
            0.80f32,
            vec!["WAL journal mode enabled", "busy_timeout set to 5000ms"],
            vec![
                (
                    "Execute PRAGMA journal_mode = WAL in src/db/config.rs",
                    "db_query",
                    serde_json::json!({ "query": "PRAGMA journal_mode = WAL;", "target": "strata.db" }),
                    serde_json::json!({
                        "file_path": "src/db/config.rs",
                        "pragma_result": "wal",
                        "synchronous": "NORMAL",
                        "wal_autocheckpoint_pages": 1000
                    }),
                    None,
                ),
                (
                    "Execute PRAGMA busy_timeout = 5000 in src/db/pool.rs",
                    "db_query",
                    serde_json::json!({ "query": "PRAGMA busy_timeout = 5000;", "target": "strata.db" }),
                    serde_json::json!({
                        "file_path": "src/db/pool.rs",
                        "busy_timeout_ms": 5000,
                        "lock_handling": "sqlite3_busy_handler active with exponential sleep backoff"
                    }),
                    None,
                ),
            ],
            Some("Database reliability initiative"),
        ),
        (
            "Implement R2D2 Connection Pool for Reader Connections",
            0.85f32,
            vec!["Reader connection pool initialized with 16 connections", "Zero read lock contention"],
            vec![
                (
                    "Create R2D2 pool builder in src/db/reader.rs",
                    "write_to_file",
                    serde_json::json!({ "file_path": "src/db/reader.rs", "max_readers": 16 }),
                    serde_json::json!({
                        "file_path": "src/db/reader.rs",
                        "pool_manager": "r2d2_sqlite::SqliteConnectionManager",
                        "max_size": 16,
                        "min_idle": 4,
                        "connection_timeout_ms": 3000
                    }),
                    None,
                ),
                (
                    "Benchmark concurrent read queries across 32 threads",
                    "run_command",
                    serde_json::json!({ "command": "cargo bench -p strata-storage --bench reader_contention" }),
                    serde_json::json!({
                        "stdout": "Benchmarking reader connection contention:\nthreads: 32 | queries: 100,000 | duration: 2.18s\nthroughput: 45,871 qps\nlatency p50: 0.22ms | p99: 0.68ms | p99.9: 1.2ms\nerrors: 0 (SQLITE_BUSY: 0)",
                        "exit_code": 0
                    }),
                    None,
                ),
            ],
            Some("Database reliability initiative"),
        ),
        (
            "Introduce Dedicated Single-Writer Mutex Queue",
            0.90f32,
            vec!["Single-writer serialized via tokio::sync::Mutex", "SQLITE_BUSY write errors prevented"],
            vec![
                (
                    "Wrap writer connection in Mutex in src/db/writer.rs",
                    "write_to_file",
                    serde_json::json!({ "file_path": "src/db/writer.rs", "mutex": "tokio" }),
                    serde_json::json!({
                        "file_path": "src/db/writer.rs",
                        "writer_struct": "Arc<tokio::sync::Mutex<rusqlite::Connection>>",
                        "transaction_isolation": "IMMEDIATE"
                    }),
                    None,
                ),
                (
                    "Implement queued write transaction channel in src/db/queue.rs",
                    "write_to_file",
                    serde_json::json!({ "file_path": "src/db/queue.rs", "capacity": 1000 }),
                    serde_json::json!({
                        "file_path": "src/db/queue.rs",
                        "channel_type": "tokio::sync::mpsc::channel(1000)",
                        "batch_window_ms": 5,
                        "max_batch_size": 50
                    }),
                    None,
                ),
            ],
            Some("Database reliability initiative"),
        ),
        (
            // Target failure episode!
            "Simulate Connection Exhaustion Under Concurrent Load",
            0.95f32,
            vec!["Pool exhaustion failure observed", "Deadlock timeout in writer lease acquire"],
            vec![
                (
                    "Spawn 100 concurrent workers requesting write leases in src/db/pool.rs",
                    "run_command",
                    serde_json::json!({ "command": "cargo test -p strata-storage --test stress_pool_concurrency", "workers": 100 }),
                    serde_json::json!({
                        "stdout": "Spawning 100 concurrent write worker tasks requesting immediate exclusive locks...\nWorker 0: acquire ok\nWorker 1: acquire ok\nWorker 2: waiting in queue...",
                        "active_leases": 1,
                        "pending_waiters": 99,
                        "file_path": "src/db/pool.rs"
                    }),
                    None,
                ),
                (
                    "Acquire write connection lease under saturated lock queue",
                    "db_query",
                    serde_json::json!({ "action": "acquire_write_lease", "worker_id": 47, "timeout_ms": 5000 }),
                    serde_json::json!({
                        "file_path": "src/db/pool.rs",
                        "error_code": "SQLITE_BUSY_TIMEOUT",
                        "trace": "rusqlite::Error::SqliteFailure(ffi::Error { code: ExtendedError(5), extended_code: 517 }, Some(\"database is locked\"))"
                    }),
                    Some("DeadlockTimeout: SQLite connection pool exhausted under concurrent writer load"),
                ),
            ],
            Some("Database reliability stress harness"),
        ),
        (
            "Tune SQLite Connection Pool Limits and Backoff Policy",
            0.85f32,
            vec!["Exponential backoff with jitter applied", "Pool recovery verified under load"],
            vec![
                (
                    "Implement jittered backoff retry in src/db/backoff.rs",
                    "write_to_file",
                    serde_json::json!({ "file_path": "src/db/backoff.rs", "max_retries": 5 }),
                    serde_json::json!({
                        "file_path": "src/db/backoff.rs",
                        "algorithm": "DecorrelatedJitter",
                        "base_delay_ms": 25,
                        "max_delay_ms": 500
                    }),
                    None,
                ),
                (
                    "Adjust writer queue capacity to 500 in src/db/pool.rs",
                    "replace_file_content",
                    serde_json::json!({ "file_path": "src/db/pool.rs", "capacity": 500 }),
                    serde_json::json!({
                        "status": "success",
                        "file_path": "src/db/pool.rs",
                        "backpressure_action": "Drop or return HTTP 503 Service Unavailable when queue reaches 500"
                    }),
                    None,
                ),
            ],
            Some("Database reliability initiative"),
        ),
        (
            "Add Prometheus Metrics for Pool Wait Time and Active Leases",
            0.75f32,
            vec!["Metrics sqlite_pool_active_connections exported", "Alert rule for queue depth registered"],
            vec![
                (
                    "Register IntGaugeVec in src/db/metrics.rs",
                    "write_to_file",
                    serde_json::json!({ "file_path": "src/db/metrics.rs", "metric": "sqlite_pool_active" }),
                    serde_json::json!({
                        "file_path": "src/db/metrics.rs",
                        "metrics_registered": [
                            "sqlite_pool_active_readers",
                            "sqlite_pool_writer_locked",
                            "sqlite_pool_queue_depth",
                            "sqlite_pool_wait_duration_seconds"
                        ]
                    }),
                    None,
                ),
                (
                    "Hook metrics into pool acquire/release in src/telemetry/prometheus.rs",
                    "replace_file_content",
                    serde_json::json!({ "file_path": "src/telemetry/prometheus.rs", "endpoint": "/metrics" }),
                    serde_json::json!({
                        "status": "success",
                        "file_path": "src/telemetry/prometheus.rs",
                        "prometheus_exporter_port": 9090,
                        "scrape_interval": "5s"
                    }),
                    None,
                ),
            ],
            Some("Database reliability initiative"),
        ),
    ];

    for (i, (summary, salience, outcomes, steps, prov)) in sqlite_defs.into_iter().enumerate() {
        let ep_id = deterministic_uuid(0x2000, i);
        let mut ep_steps = Vec::new();
        for (s_idx, (action, tool, inp_val, out_val, err_opt)) in steps.into_iter().enumerate() {
            let mut step = EpisodeStep::new(s_idx as u32, action)
                .with_tool(tool)
                .with_input(inp_val)
                .with_output(out_val);
            if let Some(err) = err_opt {
                step = step.with_error(err);
            }
            ep_steps.push(step);
        }
        let outcomes_vec: Vec<String> = outcomes.into_iter().map(String::from).collect();
        let mut ep = EpisodeRecord::new(session_id, "agent-db-lead", summary)
            .with_salience(salience)
            .with_task_id("task-sqlite-pool")
            .with_steps(ep_steps)
            .with_outcomes(outcomes_vec);
        ep.id = ep_id;
        ep.created_at = base_time + 1000 + (i as i64 * 60);
        if let Some(p) = prov {
            ep = ep.with_provenance(p);
        }
        episodes.push(ep);
    }

    // -------------------------------------------------------------------------
    // Cluster 3: CDC Outbox Worker (6 episodes)
    // -------------------------------------------------------------------------
    let cdc_defs: Vec<EpisodeDef> = vec![
        (
            "Create Outbox Events Table with Sequence Number",
            0.80f32,
            vec![
                "Table outbox_events created with BIGSERIAL id",
                "Payload stored as JSONB",
            ],
            vec![
                (
                    "Apply migration in migrations/003_outbox.sql",
                    "db_query",
                    serde_json::json!({ "file_path": "migrations/003_outbox.sql", "applied": true }),
                    serde_json::json!({
                        "file_path": "migrations/003_outbox.sql",
                        "ddl": "CREATE TABLE outbox_events (id BIGSERIAL PRIMARY KEY, event_type VARCHAR(128) NOT NULL, payload JSONB NOT NULL, status VARCHAR(32) NOT NULL DEFAULT 'pending', created_at TIMESTAMP WITH TIME ZONE DEFAULT NOW());",
                        "indexes_created": ["idx_outbox_events_status_id", "idx_outbox_events_created_at"]
                    }),
                    None,
                ),
                (
                    "Define OutboxEvent struct in src/cdc/schema.rs",
                    "write_to_file",
                    serde_json::json!({ "file_path": "src/cdc/schema.rs", "struct": "OutboxEvent" }),
                    serde_json::json!({
                        "file_path": "src/cdc/schema.rs",
                        "struct_definition": "pub struct OutboxEvent {\n    pub id: i64,\n    pub event_type: String,\n    pub payload: serde_json::Value,\n    pub status: OutboxStatus,\n    pub created_at: DateTime<Utc>,\n}"
                    }),
                    None,
                ),
            ],
            Some("Event-driven architecture overhaul"),
        ),
        (
            "Implement Transactional Outbox Event Publisher",
            0.85f32,
            vec![
                "Domain events committed atomically with entity state",
                "Publisher helper unit-tested",
            ],
            vec![
                (
                    "Implement publish_event in src/cdc/publisher.rs",
                    "write_to_file",
                    serde_json::json!({ "file_path": "src/cdc/publisher.rs", "atomic": true }),
                    serde_json::json!({
                        "file_path": "src/cdc/publisher.rs",
                        "sql_statement": "INSERT INTO outbox_events (event_type, payload, status) VALUES ($1, $2, 'pending') RETURNING id;",
                        "transactional": "Enforces execution within parent db transaction scope"
                    }),
                    None,
                ),
                (
                    "Add domain events in src/cdc/events.rs",
                    "replace_file_content",
                    serde_json::json!({ "file_path": "src/cdc/events.rs", "events": ["UserCreated", "OrderPlaced"] }),
                    serde_json::json!({
                        "file_path": "src/cdc/events.rs",
                        "events_mapped": ["UserCreatedEvent", "OrderPlacedEvent", "PaymentAuthorizedEvent", "ShipmentScheduledEvent"],
                        "serialization": "Deterministic Canonical JSON format"
                    }),
                    None,
                ),
            ],
            Some("Event-driven architecture overhaul"),
        ),
        (
            "Implement Polling CDC Engine with Batch Fetch",
            0.80f32,
            vec![
                "Poll loop fetches 50 pending events per tick",
                "Row-level locking with SKIP LOCKED",
            ],
            vec![
                (
                    "Implement batch query in src/cdc/poller.rs with SELECT FOR UPDATE SKIP LOCKED",
                    "write_to_file",
                    serde_json::json!({ "file_path": "src/cdc/poller.rs", "batch_size": 50 }),
                    serde_json::json!({
                        "file_path": "src/cdc/poller.rs",
                        "query_template": "SELECT id, event_type, payload FROM outbox_events WHERE status = 'pending' ORDER BY id ASC LIMIT 50 FOR UPDATE SKIP LOCKED;",
                        "skip_locked_benefit": "Zero lock contention between multiple parallel worker instances"
                    }),
                    None,
                ),
                (
                    "Add batch processor in src/cdc/batch.rs",
                    "write_to_file",
                    serde_json::json!({ "file_path": "src/cdc/batch.rs", "workers": 4 }),
                    serde_json::json!({
                        "file_path": "src/cdc/batch.rs",
                        "worker_concurrency": 4,
                        "batch_timeout_ms": 200,
                        "buffer_channel_size": 256
                    }),
                    None,
                ),
            ],
            Some("Event-driven architecture overhaul"),
        ),
        (
            "Add Idempotent Kafka Delivery with Ack Receipts",
            0.90f32,
            vec![
                "Kafka producer configured with acks=all",
                "Deduplication key set to event UUID",
            ],
            vec![
                (
                    "Configure rdkafka producer in src/cdc/kafka.rs",
                    "write_to_file",
                    serde_json::json!({ "file_path": "src/cdc/kafka.rs", "acks": "all" }),
                    serde_json::json!({
                        "file_path": "src/cdc/kafka.rs",
                        "client_config": {
                            "bootstrap.servers": "kafka-cluster:9092",
                            "enable.idempotence": "true",
                            "acks": "all",
                            "compression.type": "zstd",
                            "max.in.flight.requests.per.connection": 1
                        }
                    }),
                    None,
                ),
                (
                    "Send batch to topic orders.v1 with message key and verify acks",
                    "run_command",
                    serde_json::json!({ "command": "cargo test -p strata-cdc --test kafka_delivery" }),
                    serde_json::json!({
                        "stdout": "Delivering batch of 50 events to topic orders.v1:\nBroker partition 0: ack received (offset 1024..1048)\nBroker partition 1: ack received (offset 512..536)\nTotal elapsed: 18.4ms | All 50 partition acks committed.",
                        "exit_code": 0,
                        "file_path": "src/cdc/ack.rs"
                    }),
                    None,
                ),
            ],
            Some("Event-driven architecture overhaul"),
        ),
        (
            "Implement Dead Letter Queue for Poison Pill Events",
            0.85f32,
            vec![
                "DLQ routing active after 3 failed retries",
                "Poison pill isolation confirmed",
            ],
            vec![
                (
                    "Add retry counter and max_retries check in src/cdc/retry.rs",
                    "write_to_file",
                    serde_json::json!({ "file_path": "src/cdc/retry.rs", "max_retries": 3 }),
                    serde_json::json!({
                        "file_path": "src/cdc/retry.rs",
                        "backoff_sequence_secs": [1, 5, 25],
                        "circuit_breaker": "Trip after 10 consecutive failures"
                    }),
                    None,
                ),
                (
                    "Route unprocessable events to src/cdc/dlq.rs",
                    "replace_file_content",
                    serde_json::json!({ "file_path": "src/cdc/dlq.rs", "dlq_table": "outbox_dlq" }),
                    serde_json::json!({
                        "status": "success",
                        "file_path": "src/cdc/dlq.rs",
                        "dlq_action": "INSERT INTO outbox_dlq (original_id, error_reason, failed_payload) VALUES ($1, $2, $3)",
                        "notification": "Send PagerDuty webhook alert on DLQ insert"
                    }),
                    None,
                ),
            ],
            Some("Event-driven architecture overhaul"),
        ),
        (
            "Add Graceful Shutdown and Checkpointing on SIGTERM",
            0.75f32,
            vec![
                "Worker drains in-flight batches before exit",
                "Zero message loss verified on kill",
            ],
            vec![
                (
                    "Listen for tokio::signal::ctrl_c in src/cdc/worker.rs",
                    "write_to_file",
                    serde_json::json!({ "file_path": "src/cdc/worker.rs", "signal": "SIGTERM" }),
                    serde_json::json!({
                        "file_path": "src/cdc/worker.rs",
                        "shutdown_drain_timeout_ms": 10000,
                        "cancellation_token": "tokio_util::sync::CancellationToken"
                    }),
                    None,
                ),
                (
                    "Persist checkpoint sequence to src/cdc/checkpoint.rs",
                    "db_query",
                    serde_json::json!({ "file_path": "src/cdc/checkpoint.rs", "seq": 98450 }),
                    serde_json::json!({
                        "file_path": "src/cdc/checkpoint.rs",
                        "committed_sequence": 98450,
                        "unprocessed_backlog": 0,
                        "clean_shutdown": true
                    }),
                    None,
                ),
            ],
            Some("Event-driven architecture overhaul"),
        ),
    ];

    for (i, (summary, salience, outcomes, steps, prov)) in cdc_defs.into_iter().enumerate() {
        let ep_id = deterministic_uuid(0x3000, i);
        let mut ep_steps = Vec::new();
        for (s_idx, (action, tool, inp_val, out_val, err_opt)) in steps.into_iter().enumerate() {
            let mut step = EpisodeStep::new(s_idx as u32, action)
                .with_tool(tool)
                .with_input(inp_val)
                .with_output(out_val);
            if let Some(err) = err_opt {
                step = step.with_error(err);
            }
            ep_steps.push(step);
        }
        let outcomes_vec: Vec<String> = outcomes.into_iter().map(String::from).collect();
        let mut ep = EpisodeRecord::new(session_id, "agent-cdc-lead", summary)
            .with_salience(salience)
            .with_task_id("task-cdc-outbox")
            .with_steps(ep_steps)
            .with_outcomes(outcomes_vec);
        ep.id = ep_id;
        ep.created_at = base_time + 2000 + (i as i64 * 60);
        if let Some(p) = prov {
            ep = ep.with_provenance(p);
        }
        episodes.push(ep);
    }

    (episodes, target_failure_ep_id, target_failure_step_index)
}

/// Evaluation Scenario: Hierarchical Episodic Memory Pyramid Benchmark
///
/// Evaluates:
/// 1. 18 realistic multi-step episodes across 3 architectural clusters.
/// 2. Hierarchical aggregation into Tier 2 milestone rollups.
/// 3. Strict mandatory assertions:
///    - **Memory Compression**: >= 70% reduction in tokens between raw Tier 1 event traces and Tier 2 Rollups.
///    - **Drill-Down Precision**: 100% precision in locating the exact episode and step of a specific failure.
///    - **Determinism**: 100% identical UUIDs, summaries, saliences, and metrics on independent re-execution.
pub async fn run_hierarchical_episodic_pyramid_eval() -> Result<HierarchicalPyramidEvalReport> {
    println!("\n================================================================================");
    println!("🏛️  STRATA BENCHMARK — HIERARCHICAL EPISODIC MEMORY PYRAMID EVALUATION");
    println!("================================================================================");

    let session_id = "sess-eval-pyramid-benchmark";

    // 1. Generate realistic episodes
    let (episodes, expected_fail_ep_id, expected_fail_step_idx) =
        build_synthetic_pyramid_episodes(session_id);
    let total_episodes = episodes.len();
    let total_steps: usize = episodes.iter().map(|e| e.event_trace.len()).sum();

    println!(
        "▶ Total Synthetic Episodes: {} across 3 clusters",
        total_episodes
    );
    println!("▶ Total Raw Event Trace Steps: {}", total_steps);

    // Compute raw Tier 1 tokens across all steps
    let raw_tier1_tokens: usize = episodes
        .iter()
        .flat_map(|e| &e.event_trace)
        .map(estimate_step_tokens)
        .sum();

    println!(
        "▶ Raw Tier 1 Event Trace Token Footprint: ~{} tokens",
        raw_tier1_tokens
    );

    // 2. Open Store 1 and write episodes
    let store1 = Arc::new(SqliteStore::open_in_memory()?);
    for ep in &episodes {
        store1.write_episode(ep)?;
    }

    let config = PyramidConfig::new()
        .with_min_cluster_size(2)
        .with_max_cluster_size(20)
        .with_cluster_similarity_threshold(0.70);

    // 3. Run Aggregation on Store 1
    let t0 = Instant::now();
    let rollups1 =
        EpisodicPyramidEngine::aggregate_session_rollups(&store1, Some(session_id), &config, None)?;
    let latency_ms = t0.elapsed().as_secs_f64() * 1000.0;

    println!("▶ Generated Tier 2 Milestone Rollups: {}", rollups1.len());
    println!("▶ Aggregation Latency: {:.2}ms", latency_ms);

    if rollups1.len() != 3 {
        bail!(
            "Expected exactly 3 milestone rollups (1 per cluster), got {}",
            rollups1.len()
        );
    }

    // -------------------------------------------------------------------------
    // Assertion 1: Memory Compression (>= 85%)
    // -------------------------------------------------------------------------
    let rollup_tier2_tokens: usize = rollups1.iter().map(estimate_rollup_tokens).sum();
    let compression_ratio = 1.0 - (rollup_tier2_tokens as f64 / raw_tier1_tokens as f64);
    let compression_pct = compression_ratio * 100.0;

    println!("\n📊 [Assertion 1: Memory Compression]");
    println!(
        "  • Raw Tier 1 Steps Tokens:    ~{} tokens",
        raw_tier1_tokens
    );
    println!(
        "  • Tier 2 Rollups Tokens:      ~{} tokens",
        rollup_tier2_tokens
    );
    println!(
        "  • Memory Compression Ratio:   {:.1}% (Target: >= 85.0%)",
        compression_pct
    );

    if compression_pct < 85.0 {
        bail!(
            "Memory compression failed: achieved {:.2}%, required >= 85.0%",
            compression_pct
        );
    }
    println!(
        "  ✓ Memory compression assertion PASSED ({:.1}% >= 85%)",
        compression_pct
    );

    // -------------------------------------------------------------------------
    // Assertion 2: Drill-Down Accuracy (100% Precision)
    // -------------------------------------------------------------------------
    println!("\n🔍 [Assertion 2: Drill-Down Precision]");

    // Locate the SQLite pool rollup
    let pool_rollup = rollups1
        .iter()
        .find(|r| r.task_id.as_deref() == Some("task-sqlite-pool"))
        .ok_or_else(|| anyhow::anyhow!("SQLite pool rollup not found"))?;

    println!(
        "  • Inspecting Rollup: ID {} ({})",
        pool_rollup.id, pool_rollup.summary
    );

    let tree = EpisodicPyramidEngine::drill_down(&store1, &pool_rollup.id)?
        .ok_or_else(|| anyhow::anyhow!("DrillDownTree not found for rollup {}", pool_rollup.id))?;

    println!(
        "  • DrillDownTree: {} leaves, {} total steps",
        tree.leaves.len(),
        tree.total_steps
    );

    // Filter tree leaves for failures
    let failing_leaves: Vec<&EpisodeRecord> = tree
        .leaves
        .iter()
        .filter(|ep| ep.event_trace.iter().any(|s| s.error.is_some()))
        .collect();

    if failing_leaves.len() != 1 {
        bail!(
            "Drill-down precision failed: expected exactly 1 failing leaf episode, got {}",
            failing_leaves.len()
        );
    }

    let found_fail_ep = failing_leaves[0];
    let failing_steps: Vec<&EpisodeStep> = found_fail_ep
        .event_trace
        .iter()
        .filter(|s| s.error.is_some())
        .collect();

    if failing_steps.len() != 1 {
        bail!(
            "Drill-down precision failed: expected exactly 1 failing step in episode, got {}",
            failing_steps.len()
        );
    }

    let found_fail_step = failing_steps[0];

    // Assert exact match on episode ID, step index, and error context
    if found_fail_ep.id != expected_fail_ep_id {
        bail!(
            "Drill-down targeted wrong episode: expected {}, got {}",
            expected_fail_ep_id,
            found_fail_ep.id
        );
    }

    if found_fail_step.step_index != expected_fail_step_idx {
        bail!(
            "Drill-down targeted wrong step index: expected {}, got {}",
            expected_fail_step_idx,
            found_fail_step.step_index
        );
    }

    let err_msg = found_fail_step.error.as_deref().unwrap_or("");
    if !err_msg.contains("DeadlockTimeout") {
        bail!("Drill-down step error content mismatch: '{}'", err_msg);
    }

    let drill_down_accuracy = 1.0f64;
    println!(
        "  • Pinpointed Failure Episode: {} ({})",
        found_fail_ep.id, found_fail_ep.summary
    );
    println!(
        "  • Pinpointed Step Index:      Step {} -> {}",
        found_fail_step.step_index, found_fail_step.action
    );
    println!("  • Error Identified:           {}", err_msg);
    println!("  ✓ Drill-down accuracy assertion PASSED (100.0% precision)");

    // -------------------------------------------------------------------------
    // Assertion 3: Determinism (Exact Reproducibility)
    // -------------------------------------------------------------------------
    println!("\n🔄 [Assertion 3: Determinism across Store Instances]");

    let store2 = Arc::new(SqliteStore::open_in_memory()?);
    for ep in &episodes {
        store2.write_episode(ep)?;
    }

    let rollups2 =
        EpisodicPyramidEngine::aggregate_session_rollups(&store2, Some(session_id), &config, None)?;

    if rollups1.len() != rollups2.len() {
        bail!(
            "Determinism mismatch: store 1 produced {} rollups, store 2 produced {}",
            rollups1.len(),
            rollups2.len()
        );
    }

    for (i, (r1, r2)) in rollups1.iter().zip(&rollups2).enumerate() {
        if r1.id != r2.id {
            bail!(
                "Determinism mismatch on rollup {i} ID: {} vs {}",
                r1.id,
                r2.id
            );
        }
        if r1.summary != r2.summary {
            bail!(
                "Determinism mismatch on rollup {i} summary: '{}' vs '{}'",
                r1.summary,
                r2.summary
            );
        }
        if (r1.salience_score - r2.salience_score).abs() > 1e-5 {
            bail!(
                "Determinism mismatch on rollup {i} salience: {} vs {}",
                r1.salience_score,
                r2.salience_score
            );
        }
        if r1.outcomes != r2.outcomes {
            bail!("Determinism mismatch on rollup {i} outcomes");
        }
        if r1.causal_provenance != r2.causal_provenance {
            bail!("Determinism mismatch on rollup {i} causal provenance");
        }
        if r1.child_ids != r2.child_ids {
            bail!("Determinism mismatch on rollup {i} child IDs");
        }
        if r1.created_at != r2.created_at {
            bail!("Determinism mismatch on rollup {i} created_at");
        }
    }

    println!("  • Store 1 and Store 2 generated IDENTICAL rollups:");
    for r in &rollups1 {
        println!(
            "    - ID: {} | Salience: {:.2} | Children: {} | Summary: {}",
            r.id,
            r.salience_score,
            r.child_ids.len(),
            r.summary
        );
    }
    println!("  ✓ Determinism assertion PASSED (100% identical outputs)");

    // -------------------------------------------------------------------------
    // Assertion 4: Progressive Drill-Down & Temporal Filtering (EpisodicPyramidStore)
    // -------------------------------------------------------------------------
    println!("\n🔍 [Assertion 4: Progressive Drill-Down & Temporal Filtering]");

    let pyramid_store = EpisodicPyramidStore::new(store1.clone()).with_config(config.clone());

    // 4a. Compact view: expand_episodes = false
    let compact_filter = PyramidFilter::new().with_session_id(session_id);
    let compact_report = pyramid_store.drill_down_progressive(&compact_filter, false)?;

    if compact_report.expanded {
        bail!("Expected compact report to have expanded = false");
    }
    if compact_report.rollups_count != 3 {
        bail!(
            "Expected 3 rollups in compact report, got {}",
            compact_report.rollups_count
        );
    }
    if compact_report.total_steps != 0 {
        bail!(
            "Expected zero execution steps emitted in compact report, got {}",
            compact_report.total_steps
        );
    }
    if !compact_report.trees.is_empty() {
        bail!("Expected empty trees in compact report");
    }
    println!(
        "  • Compact Tier 2 View: 3 rollups, 0 raw steps emitted (Tokens preserved: {:.1}%)",
        compression_pct
    );

    // 4b. On-demand expansion: expand_episodes = true for task-sqlite-pool
    let expand_filter = PyramidFilter::new()
        .with_session_id(session_id)
        .with_cluster_id("task-sqlite-pool");
    let expanded_report = pyramid_store.drill_down_progressive(&expand_filter, true)?;

    if !expanded_report.expanded {
        bail!("Expected expanded report to have expanded = true");
    }
    if expanded_report.rollups_count != 1 {
        bail!(
            "Expected 1 rollup for task-sqlite-pool, got {}",
            expanded_report.rollups_count
        );
    }
    if expanded_report.trees.len() != 1 {
        bail!(
            "Expected 1 tree for task-sqlite-pool, got {}",
            expanded_report.trees.len()
        );
    }

    let sqlite_tree = &expanded_report.trees[0];
    let failing_leaves_exp: Vec<&EpisodeRecord> = sqlite_tree
        .leaves
        .iter()
        .filter(|ep| ep.event_trace.iter().any(|s| s.error.is_some()))
        .collect();

    if failing_leaves_exp.len() != 1 {
        bail!(
            "Expected exactly 1 failing leaf in on-demand expansion, got {}",
            failing_leaves_exp.len()
        );
    }
    let fail_ep = failing_leaves_exp[0];
    if fail_ep.id != expected_fail_ep_id {
        bail!(
            "On-demand expansion failed to pinpoint expected failure episode: {} vs {}",
            fail_ep.id,
            expected_fail_ep_id
        );
    }
    let fail_step = fail_ep
        .event_trace
        .iter()
        .find(|s| s.error.is_some())
        .unwrap();
    if fail_step.step_index != expected_fail_step_idx {
        bail!(
            "On-demand expansion failed to pinpoint step index: {} vs {}",
            fail_step.step_index,
            expected_fail_step_idx
        );
    }
    println!("  • On-demand Progressive Expansion: Pinpointed failing episode {} (Step {}) with 100% precision", fail_ep.id, fail_step.step_index);

    // 4c. Temporal interval filtering
    // Episodes: auth (1700000000..1700000300), sqlite (1700001000..1700001300), cdc (1700002000..1700002300)
    let time_filter = PyramidFilter::new()
        .with_session_id(session_id)
        .with_start_time(1700000000 + 800)
        .with_end_time(1700000000 + 1600);
    let time_report = pyramid_store.drill_down_progressive(&time_filter, false)?;
    if time_report.rollups_count != 1 {
        bail!(
            "Expected temporal filter to isolate exactly 1 rollup (SQLite pool), got {}",
            time_report.rollups_count
        );
    }
    if time_report.summaries[0].task_id.as_deref() != Some("task-sqlite-pool") {
        bail!(
            "Expected isolated rollup to be task-sqlite-pool, got {:?}",
            time_report.summaries[0].task_id
        );
    }
    println!(
        "  • Temporal Interval Filtering: Accurately isolated cluster '{}' across timestamp window",
        time_report.summaries[0].task_id.as_deref().unwrap_or("")
    );

    println!("  ✓ Progressive drill-down and temporal filtering assertion PASSED (100% precision)");

    println!("\n================================================================================");
    println!("🎉 HIERARCHICAL EPISODIC MEMORY PYRAMID EVALUATION PASSED (ALL 4/4 ASSERTIONS)");
    println!("================================================================================\n");

    Ok(HierarchicalPyramidEvalReport {
        raw_tier1_tokens,
        rollup_tier2_tokens,
        compression_ratio,
        drill_down_accuracy,
        is_deterministic: true,
        clusters_count: rollups1.len(),
        total_episodes,
        total_steps,
        latency_ms,
    })
}

/// Scenario wrapper returning Result<()> for eval runner integration
pub async fn run_hierarchical_episodic_pyramid_scenario() -> Result<()> {
    run_hierarchical_episodic_pyramid_eval().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_eval_hierarchical_episodic_pyramid() {
        let report = run_hierarchical_episodic_pyramid_eval()
            .await
            .expect("Hierarchical episodic pyramid eval failed");
        assert!(
            report.compression_ratio >= 0.85,
            "Compression ratio must be >= 0.85 (85%+), was {:.2}",
            report.compression_ratio
        );
        assert_eq!(
            report.drill_down_accuracy, 1.0,
            "Drill down accuracy must be 1.0 (100%)"
        );
        assert!(
            report.is_deterministic,
            "Pyramid aggregation must be deterministic"
        );
        assert_eq!(report.clusters_count, 3, "Expected 3 milestone clusters");
    }
}
