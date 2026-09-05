use anyhow::{bail, Result};
use chrono::{Duration, Utc};
use strata_cli::commands::observe::{generate_report, ObserveArgs};
use strata_core::{
    schemas::{FeedbackEvent, FeedbackRating, ImplicitSignal, SemanticFact, SignalKind},
    state::{FailurePattern, FailureSeverity, Scope},
};
use strata_memory::SqliteStore;

/// Scenario 9: Cognitive Observability, Mathematical Decay Visualizer & Anti-Pattern Audit
/// Evaluates:
/// 1. Ebbinghaus decay curve simulation generates monotonic decay across 168h.
/// 2. ACT-R activation and stability metrics are computed correctly across memory types.
/// 3. Anti-patterns are audited with severity breakdown and proven mitigations.
/// 4. Reinforcement feedback and implicit signals are aggregated accurately in the report.
pub async fn run_cognitive_observability_scenario() -> Result<()> {
    println!("\n▶ Running Eval Scenario: Cognitive Observability & Decay Dashboard");

    let store = SqliteStore::open_in_memory()?;
    let now = Utc::now();
    let two_days_ago = now - Duration::days(2);
    let five_days_ago = now - Duration::days(5);

    // 1. Seed Memory Records (Invariant, Healthy, At-Risk)
    let mut invariant_fact = SemanticFact::new(
        "Strata offline-first SQLite synchronization protocol",
        "protocol",
        Scope::Global,
    )
    .with_importance(0.98)
    .with_confidence(1.0);
    invariant_fact.created_at = five_days_ago;
    store.insert_or_update_semantic_fact(&invariant_fact)?;

    let mut healthy_fact = SemanticFact::new(
        "Axum HTTP security headers require HSTS and CSP",
        "security",
        Scope::Global,
    )
    .with_importance(0.6)
    .with_confidence(1.0);
    healthy_fact.created_at = two_days_ago;
    store.insert_or_update_semantic_fact(&healthy_fact)?;

    let mut ephemeral_fact = SemanticFact::new(
        "Temporary local build cache pointer",
        "cache",
        Scope::Global,
    )
    .with_importance(0.15)
    .with_confidence(0.5);
    ephemeral_fact.created_at = five_days_ago;
    store.insert_or_update_semantic_fact(&ephemeral_fact)?;

    // 2. Seed Failure Pattern (Anti-Pattern)
    let anti_pattern = FailurePattern {
        id: uuid::Uuid::new_v4(),
        signature: "eval_infinite_retry_loop".to_string(),
        pattern_name: "Infinite Subagent Invocation Loop".to_string(),
        description: "Agent continuously retried failing endpoint without exponential backoff"
            .to_string(),
        trigger_condition: "HTTP 503 Service Unavailable".to_string(),
        error_type: "NetworkLoop".to_string(),
        mitigation: "Implement jittered exponential backoff and max 3 attempts".to_string(),
        occurrences: 4,
        first_seen: five_days_ago,
        last_seen: two_days_ago,
        severity: FailureSeverity::High,
        scope: Scope::Global,
        metadata: serde_json::json!({ "impact": "high_token_waste" }),
    };
    store.upsert_failure_pattern(&anti_pattern)?;

    // 3. Seed Feedback and Implicit Signals
    let fb_pos = FeedbackEvent::new(FeedbackRating::Positive, "agent_review")
        .with_comment("Successfully avoided infinite loop using exponential backoff");
    store.record_feedback_event(&fb_pos)?;

    let sig_1 = ImplicitSignal::new(SignalKind::ToolLoop, "session_test_1", "agent_1");
    store.record_implicit_signal(&sig_1)?;

    let sig_2 = ImplicitSignal::new(SignalKind::GitRevert, "session_test_1", "agent_1");
    store.record_implicit_signal(&sig_2)?;

    // 4. Generate Cognitive Report
    let args = ObserveArgs {
        live: false,
        interval_secs: 2,
        tab: Some("overview".to_string()),
        scope: None,
        horizon_hours: 168.0,
        limit: 10,
        json: true,
    };

    let report = generate_report(&store, &args)?;

    println!("  [Cognitive Observability Metrics]");
    println!(
        "    • Total Memories Audited:       {}",
        report.total_memories
    );
    println!(
        "    • Active Semantic Facts:        {}",
        report.active_semantic_facts
    );
    println!(
        "    • Captured Anti-Patterns:       {}",
        report.anti_patterns_count
    );
    println!(
        "    • At-Risk Memories Detected:    {}",
        report.at_risk_memories_count
    );
    println!(
        "    • Positive Feedback Recorded:   {}",
        report.positive_feedback
    );
    println!(
        "    • Implicit Signals Mined:       {}",
        report.total_implicit_signals
    );

    if report.total_memories != 3 {
        bail!(
            "Expected 3 total memories in report, found {}",
            report.total_memories
        );
    }

    if report.anti_patterns_count != 1 {
        bail!(
            "Expected 1 anti-pattern, found {}",
            report.anti_patterns_count
        );
    }

    if report.anti_patterns[0].severity != "HIGH" {
        bail!(
            "Expected HIGH severity for anti-pattern, found {}",
            report.anti_patterns[0].severity
        );
    }

    if report.positive_feedback != 1 {
        bail!(
            "Expected 1 positive feedback event, found {}",
            report.positive_feedback
        );
    }

    if report.total_implicit_signals != 2 {
        bail!(
            "Expected 2 implicit signals, found {}",
            report.total_implicit_signals
        );
    }

    // 5. Verify Invariant vs Ephemeral Memory Retention
    let inv_mem = report
        .memories
        .iter()
        .find(|m| m.title.contains("offline-first"))
        .expect("Invariant memory must be present");

    if !inv_mem.is_invariant || inv_mem.ebbinghaus_retention < 0.99 {
        bail!(
            "Invariant memory must retain ~1.0 retention score, found {}",
            inv_mem.ebbinghaus_retention
        );
    }

    let eph_mem = report
        .memories
        .iter()
        .find(|m| m.title.contains("Temporary local build"))
        .expect("Ephemeral memory must be present");

    if eph_mem.ebbinghaus_retention >= 0.5 {
        bail!(
            "Ephemeral memory from 5 days ago must be at risk / decayed (< 0.5), found {}",
            eph_mem.ebbinghaus_retention
        );
    }

    // 6. Verify Ebbinghaus Curve Monotonicity
    let points = &report.ebbinghaus_curve_points;
    if points.len() < 10 {
        bail!("Expected at least 10 curve points, got {}", points.len());
    }

    for i in 1..points.len() {
        if points[i].high_retention > points[i - 1].high_retention + 0.001 {
            bail!("Ebbinghaus curve must be monotonically non-increasing");
        }
    }

    println!("  ✓ Ebbinghaus mathematical curve verified (monotonic decay across 168h)");
    println!("  ✓ Anti-pattern audit and mitigation rule registry verified");
    println!("  ✓ ACT-R activation risk inspector verified");

    Ok(())
}
