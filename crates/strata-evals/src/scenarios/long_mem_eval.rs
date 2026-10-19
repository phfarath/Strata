use anyhow::{bail, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::time::Instant;

use strata_core::{
    events::Provenance,
    schemas::FactStatus,
    state::{MemoryRecord, MemoryType, Scope},
    traits::MemoryEngine,
};
use strata_memory::{
    DecayCalculator, EmbeddingProvider, MockEmbeddingProvider, SqliteMemoryEngine,
    TruthMaintenanceSystem,
};

/// Recall metrics for a specific interaction cycle horizon (10, 100, 1000).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CycleRecallMetrics {
    pub cycle: usize,
    pub queries_tested: usize,
    pub correct_retrievals: usize,
    pub recall_accuracy: f32,
    pub average_latency_ms: f32,
    pub contradiction_resolution_accuracy: f32,
}

/// Evaluation report summarizing LongMemEval-Code benchmark results.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LongMemEvalReport {
    pub suite_name: String,
    pub metrics_10_cycles: CycleRecallMetrics,
    pub metrics_100_cycles: CycleRecallMetrics,
    pub metrics_1000_cycles: CycleRecallMetrics,
    pub cross_agent_transfer_passed: bool,
    pub contradiction_resolution_passed: bool,
    pub overall_score: f32,
}

impl LongMemEvalReport {
    /// Formats an ASCII summary table detailing the benchmark results,
    /// suitable for CLI and log presentation when evaluating coding-agents.
    pub fn format_ascii_report(&self) -> String {
        let mut out = String::new();
        out.push_str("\n========================================================================================\n");
        out.push_str(&format!(
            "📊 LONGMEMEVAL-CODE BENCHMARK REPORT — SUITE: {}\n",
            self.suite_name
        ));
        out.push_str("========================================================================================\n");
        out.push_str("Horizon (Cycles) | Queries | Correct | Recall Accuracy | Contradiction Res | Latency (ms)\n");
        out.push_str("-----------------+---------+---------+-----------------+-------------------+-------------\n");
        for m in [
            &self.metrics_10_cycles,
            &self.metrics_100_cycles,
            &self.metrics_1000_cycles,
        ] {
            out.push_str(&format!(
                "{:<16} | {:^7} | {:^7} | {:>14.1}% | {:>16.1}% | {:>9.2} ms\n",
                format!("{} Cycles", m.cycle),
                m.queries_tested,
                m.correct_retrievals,
                m.recall_accuracy * 100.0,
                m.contradiction_resolution_accuracy * 100.0,
                m.average_latency_ms
            ));
        }
        out.push_str("-----------------+---------+---------+-----------------+-------------------+-------------\n");
        out.push_str(&format!(
            "Cross-Agent Context Transfer : {}\n",
            if self.cross_agent_transfer_passed {
                "PASSED"
            } else {
                "FAILED"
            }
        ));
        out.push_str(&format!(
            "Contradiction Resolution    : {}\n",
            if self.contradiction_resolution_passed {
                "PASSED"
            } else {
                "FAILED"
            }
        ));
        out.push_str(&format!(
            "Overall Benchmark Score      : {:.1}% ({})\n",
            self.overall_score * 100.0,
            if self.overall_score >= 0.80 {
                "PASSED"
            } else {
                "FAILED"
            }
        ));
        out.push_str("========================================================================================\n");
        out
    }
}

/// Orchestrator for the LongMemEval-Code benchmark suite.
pub struct LongMemEvalSuite;

impl LongMemEvalSuite {
    /// Executes the evaluation suite by name.
    /// If `suite_name == "coding-agents"`, it executes the full LongMemEval-Code benchmark.
    pub async fn run_suite(suite_name: &str) -> Result<LongMemEvalReport> {
        if suite_name != "coding-agents" {
            bail!(
                "Unsupported eval suite name: '{}'. Supported suites: 'coding-agents'",
                suite_name
            );
        }
        run_long_mem_eval_scenario().await
    }
}

#[derive(Debug, Clone)]
struct TargetCodeRule {
    title: &'static str,
    content: &'static str,
    category: &'static str,
    query: &'static str,
    expected_keyword: &'static str,
}

fn get_target_code_rules() -> Vec<TargetCodeRule> {
    vec![
        TargetCodeRule {
            title: "Git branching workflow",
            content: "Git workflow constraint: Never commit directly to the main branch; all changes require feature branches and pull requests.",
            category: "git_workflow",
            query: "What is the git branch commit policy for the main branch?",
            expected_keyword: "branch",
        },
        TargetCodeRule {
            title: "Storage engine architecture",
            content: "Architectural constraint: Use embedded SQLite with WAL mode and FTS5 for local-first storage persistence.",
            category: "storage",
            query: "What storage engine and SQLite configuration is used for local persistence?",
            expected_keyword: "sqlite",
        },
        TargetCodeRule {
            title: "Agent concurrency leases",
            content: "Agent concurrency rule: Acquire exclusive leases via StigmergyCoordinator before modifying crate source files to prevent concurrent agent collisions.",
            category: "concurrency",
            query: "How should concurrent agents coordinate source file edits to prevent collision?",
            expected_keyword: "lease",
        },
        TargetCodeRule {
            title: "Tooling build hook",
            content: "Tooling constraint: Wrap test and build commands with strata hook wrap to capture compiler failures out-of-band.",
            category: "tooling",
            query: "How should build and test commands be executed to capture compiler failures?",
            expected_keyword: "hook",
        },
        TargetCodeRule {
            title: "Async trait macro annotation",
            content: "Workspace convention: Public async trait declarations must be annotated with the async_trait macro annotation.",
            category: "rust_conventions",
            query: "What macro annotation is required for public async trait declarations?",
            expected_keyword: "async_trait",
        },
        TargetCodeRule {
            title: "Episodic memory provenance",
            content: "Memory model rule: Episodic tier memory records require valid session provenance and timestamps.",
            category: "memory_tier",
            query: "What metadata is required for episodic tier memory records?",
            expected_keyword: "provenance",
        },
        TargetCodeRule {
            title: "Tokio worker thread pool",
            content: "Runtime architecture: Tokio multi-threaded runtime must be configured with worker threads matching system num_cpus.",
            category: "runtime",
            query: "How should the Tokio async runtime worker threads be configured?",
            expected_keyword: "tokio",
        },
        TargetCodeRule {
            title: "Error handling conventions",
            content: "Error handling standard: Cross-crate boundary functions must return StrataError or anyhow Result without using unwrap in production.",
            category: "error_handling",
            query: "What error handling conventions apply across crate boundaries?",
            expected_keyword: "strataerror",
        },
        TargetCodeRule {
            title: "Database migrations path",
            content: "Database migration rule: Database migrations must be strictly backward compatible and stored in crates/strata-memory/migrations.",
            category: "migrations",
            query: "Where must backward-compatible database migrations be placed?",
            expected_keyword: "migration",
        },
        TargetCodeRule {
            title: "HTTP network client timeout",
            content: "Network client constraint: HTTP requests must use reqwest Client configured with a 15-second connection timeout.",
            category: "networking",
            query: "What HTTP client and connection timeout must be used for network requests?",
            expected_keyword: "reqwest",
        },
    ]
}

/// Evaluates factual recall under simulated interaction cycles (with memory access logs,
/// ACT-R power-law decay, and intermittent background noise/distractor memories simulating
/// 10, 100, and 1000 agent sessions/actions).
async fn evaluate_cycle_horizon(
    cycles: usize,
    rules: &[TargetCodeRule],
) -> Result<(CycleRecallMetrics, bool)> {
    let engine = SqliteMemoryEngine::open_in_memory(None)?;
    let store = engine.store();
    let scope = Scope::Project("mem-research".to_string());
    let decay_calc = DecayCalculator::with_default_config();

    // 1. Ingest target code rules with reinforcement access logs
    for rule in rules {
        let mut mem = MemoryRecord::new(MemoryType::Semantic, rule.content, scope.clone())
            .with_summary(rule.title)
            .with_importance(0.95)
            .with_confidence(1.0)
            .with_tags(vec![rule.category.to_string()]);

        // Simulate reinforcement access logs across interaction cycles
        let access_count = cycles.min(25);
        let mut access_times = Vec::with_capacity(access_count);
        for k in 0..access_count {
            access_times
                .push(Utc::now() - chrono::Duration::hours(((access_count - k) * 2) as i64));
        }
        mem.access_count = access_count as u64;

        // Verify ACT-R decay stability for reinforced memory
        let decay_metrics = decay_calc.evaluate_decay_with_tier(
            mem.tier,
            mem.importance,
            mem.confidence,
            mem.created_at,
            &access_times,
            Utc::now(),
        );
        if decay_metrics.is_expired {
            bail!(
                "Target rule '{}' decayed prematurely under ACT-R model",
                rule.title
            );
        }
        if decay_metrics.retention < 0.80 {
            bail!(
                "Target rule '{}' retention dropped below 0.80 ({:.3})",
                rule.title,
                decay_metrics.retention
            );
        }

        engine.write(&mem).await?;
    }

    // 2. Ingest background noise / distractor memories simulating developer sessions
    for i in 0..cycles {
        let distractor_text = match i % 10 {
            0 => format!("Session #{i}: Inspected file src/temp_test.rs at line 14"),
            1 => format!("Session #{i}: Ran cargo check on branch fix-typo with exit code 0"),
            2 => format!("Session #{i}: Formatted files with cargo fmt --all"),
            3 => format!("Session #{i}: Executed git status finding 2 untracked scratch files"),
            4 => format!("Session #{i}: Temporary log output: buffer size 4096 bytes"),
            5 => format!("Session #{i}: Cleaned target debug directory for disk space"),
            6 => format!("Session #{i}: Evaluated regex pattern on user query"),
            7 => format!("Session #{i}: Downloaded mock test payload of 12 KB"),
            8 => format!("Session #{i}: Read terminal environment variable DEBUG=1"),
            _ => format!("Session #{i}: Closed ephemeral task socket connection"),
        };
        let mut distractor =
            MemoryRecord::new(MemoryType::Episodic, distractor_text, scope.clone())
                .with_importance(0.20)
                .with_confidence(0.50);
        distractor.created_at = Utc::now() - chrono::Duration::hours((cycles - i) as i64);

        store.insert_or_update_memory(&distractor)?;
    }

    // 3. Contradiction Resolution Test Case: Older vs Newer Library API Rule
    let older_api_rule = "The database network driver uses synchronous Connection::open() blocking socket call in v1 API.";
    let newer_api_rule = "The database network driver migrated to asynchronous Connection::open_async().await in v2 API, deprecating synchronous Connection::open().";

    let mut fact_old = strata_core::schemas::SemanticFact::new(
        older_api_rule,
        "database_driver_api",
        Scope::Global,
    )
    .with_importance(0.75)
    .with_confidence(0.85);
    fact_old.created_at = Utc::now() - chrono::Duration::hours(48);

    let mut fact_new = strata_core::schemas::SemanticFact::new(
        newer_api_rule,
        "database_driver_api",
        Scope::Global,
    )
    .with_importance(0.90)
    .with_confidence(0.95);
    fact_new.created_at = Utc::now();

    let embedder = MockEmbeddingProvider::default();
    let jtms = TruthMaintenanceSystem::with_default_threshold();

    let emb_old = embedder.embed_text(older_api_rule).await?;
    jtms.resolve_and_upsert(store, &mut fact_old, &emb_old)?;

    let emb_new = embedder.embed_text(newer_api_rule).await?;
    let conflicts = jtms.resolve_and_upsert(store, &mut fact_new, &emb_new)?;

    let contradiction_resolved = !conflicts.is_empty()
        && store
            .get_semantic_fact(&fact_old.id)?
            .map(|f| f.status == FactStatus::Deprecated)
            .unwrap_or(false)
        && store
            .get_semantic_fact(&fact_new.id)?
            .map(|f| f.status == FactStatus::Active)
            .unwrap_or(false);

    // 4. Test Queries for Target Rules
    let mut correct_retrievals = 0;
    let mut total_latency_ms = 0.0f32;
    let queries_tested = rules.len();

    for rule in rules {
        let t_start = Instant::now();
        let retrieved = engine.search(rule.query, Some(&scope), 5).await?;
        let latency = t_start.elapsed().as_secs_f32() * 1000.0;
        total_latency_ms += latency;

        let found = retrieved.iter().take(3).any(|m| {
            let content_lower = m.content.to_lowercase();
            content_lower.contains(&rule.expected_keyword.to_lowercase())
        });

        if found {
            correct_retrievals += 1;
        }
    }

    let recall_accuracy = if queries_tested > 0 {
        correct_retrievals as f32 / queries_tested as f32
    } else {
        0.0
    };

    let average_latency_ms = if queries_tested > 0 {
        total_latency_ms / queries_tested as f32
    } else {
        0.0
    };

    let metrics = CycleRecallMetrics {
        cycle: cycles,
        queries_tested,
        correct_retrievals,
        recall_accuracy,
        average_latency_ms,
        contradiction_resolution_accuracy: if contradiction_resolved { 1.0 } else { 0.0 },
    };

    Ok((metrics, contradiction_resolved))
}

/// Evaluates cross-agent context transfer where Agent 1 records specialized codebase
/// knowledge and Agent 2 recalls and applies it in a different session.
async fn evaluate_cross_agent_transfer() -> Result<bool> {
    let engine = SqliteMemoryEngine::open_in_memory(None)?;
    let scope = Scope::Project("mem-research".to_string());

    // Agent 1: Records specialized codebase knowledge with session provenance
    let agent_1 = "agent-architect-01";
    let session_1 = "sess-arch-spec-42";
    let mut prov_1 = Provenance::new(agent_1, session_1);
    prov_1.client = Some("antigravity".to_string());

    let specialized_content = "To handle streaming chunk responses in crate strata-memory, always use the FramedRead buffer adapter with 64KB chunk limit.";
    let mut mem_agent_1 =
        MemoryRecord::new(MemoryType::Semantic, specialized_content, scope.clone())
            .with_summary("Streaming Chunk FramedRead Buffer Adapter")
            .with_importance(0.95)
            .with_confidence(1.0)
            .with_tags(vec![
                "streaming".to_string(),
                "framed_read".to_string(),
                "buffer".to_string(),
            ]);

    let mut metadata = serde_json::Map::new();
    metadata.insert("author_agent".to_string(), serde_json::json!(agent_1));
    metadata.insert("source_session".to_string(), serde_json::json!(session_1));
    mem_agent_1.metadata = serde_json::Value::Object(metadata);

    engine.write(&mem_agent_1).await?;

    // Agent 2: Queries for streaming chunk response handling in a subsequent session
    let agent_2_query =
        "What buffer adapter should be used for streaming chunk responses in strata-memory?";
    let retrieved = engine.search(agent_2_query, Some(&scope), 3).await?;

    if retrieved.is_empty() {
        return Ok(false);
    }

    let top = &retrieved[0];
    let content_matches = top
        .content
        .contains("FramedRead buffer adapter with 64KB chunk limit");
    let author_matches = top
        .metadata
        .get("author_agent")
        .and_then(|v| v.as_str())
        .map(|a| a == agent_1)
        .unwrap_or(false);

    Ok(content_matches && author_matches)
}

/// Executes the complete LongMemEval-Code evaluation scenario.
pub async fn run_long_mem_eval_scenario() -> Result<LongMemEvalReport> {
    println!(
        "\n▶ Running Eval Scenario: LongMemEval-Code (Automated Evaluation for Coding Agents)"
    );

    let rules = get_target_code_rules();

    // 1. Evaluate 10 cycles horizon
    println!("  • Evaluating 10 interaction cycles horizon...");
    let (metrics_10, contra_10) = evaluate_cycle_horizon(10, &rules).await?;

    // 2. Evaluate 100 cycles horizon
    println!("  • Evaluating 100 interaction cycles horizon...");
    let (metrics_100, contra_100) = evaluate_cycle_horizon(100, &rules).await?;

    // 3. Evaluate 1000 cycles horizon
    println!("  • Evaluating 1000 interaction cycles horizon...");
    let (metrics_1000, contra_1000) = evaluate_cycle_horizon(1000, &rules).await?;

    let contradiction_resolution_passed = contra_10 && contra_100 && contra_1000;

    // 4. Evaluate Cross-Agent Context Transfer
    println!("  • Evaluating Cross-Agent Context Transfer...");
    let cross_agent_transfer_passed = evaluate_cross_agent_transfer().await?;

    // Compute overall score
    let overall_score = metrics_10.recall_accuracy * 0.25
        + metrics_100.recall_accuracy * 0.25
        + metrics_1000.recall_accuracy * 0.25
        + (if contradiction_resolution_passed {
            1.0
        } else {
            0.0
        }) * 0.15
        + (if cross_agent_transfer_passed {
            1.0
        } else {
            0.0
        }) * 0.10;

    if metrics_10.recall_accuracy < 0.90 {
        bail!(
            "10-cycle recall accuracy ({:.2}) failed target >= 0.90",
            metrics_10.recall_accuracy
        );
    }
    if metrics_100.recall_accuracy < 0.85 {
        bail!(
            "100-cycle recall accuracy ({:.2}) failed target >= 0.85",
            metrics_100.recall_accuracy
        );
    }
    if metrics_1000.recall_accuracy < 0.80 {
        bail!(
            "1000-cycle recall accuracy ({:.2}) failed target >= 0.80",
            metrics_1000.recall_accuracy
        );
    }
    if !contradiction_resolution_passed {
        bail!("Contradiction resolution test failed");
    }
    if !cross_agent_transfer_passed {
        bail!("Cross-agent context transfer test failed");
    }

    let report = LongMemEvalReport {
        suite_name: "coding-agents".to_string(),
        metrics_10_cycles: metrics_10,
        metrics_100_cycles: metrics_100,
        metrics_1000_cycles: metrics_1000,
        cross_agent_transfer_passed,
        contradiction_resolution_passed,
        overall_score,
    };

    println!("{}", report.format_ascii_report());

    Ok(report)
}
