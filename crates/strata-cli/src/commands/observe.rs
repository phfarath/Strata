use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;
use uuid::Uuid;

use strata_core::{
    schemas::{FactStatus, FeedbackRating},
    state::FailureSeverity,
};
use strata_memory::{DecayCalculator, SqliteStore};

/// Command arguments for the cognitive observability dashboard.
#[derive(clap::Args, Debug, Clone)]
pub struct ObserveArgs {
    /// Launch interactive terminal dashboard with auto-refreshing live view
    #[arg(long, short = 'l', help = "Live auto-refresh mode in terminal")]
    pub live: bool,

    /// Live refresh interval in seconds
    #[arg(
        long,
        default_value_t = 2,
        help = "Refresh interval in seconds for live mode"
    )]
    pub interval_secs: u64,

    /// Filter view to specific tab: 'overview', 'decay', 'anti-patterns', 'feedback'
    #[arg(
        long,
        help = "Focus view on a specific section: overview, decay, anti-patterns, feedback"
    )]
    pub tab: Option<String>,

    /// Filter by workspace or scope
    #[arg(long, help = "Filter memories by scope")]
    pub scope: Option<String>,

    /// Time horizon in hours for Ebbinghaus retention curve rendering
    #[arg(
        long,
        default_value_t = 168.0,
        help = "Horizon in hours for retention curve (e.g. 168 = 7 days)"
    )]
    pub horizon_hours: f32,

    /// Maximum rows to display in lists
    #[arg(long, default_value_t = 12, help = "Max items to display in tables")]
    pub limit: usize,

    /// Output full observability stats as JSON
    #[arg(long, help = "Output telemetry report as raw JSON")]
    pub json: bool,
}

/// Aggregated telemetry and cognitive health report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CognitiveReport {
    pub generated_at: DateTime<Utc>,
    pub total_memories: usize,
    pub active_semantic_facts: usize,
    pub deprecated_semantic_facts: usize,
    pub procedural_skills: usize,
    pub anti_patterns_count: usize,
    pub total_feedback_events: usize,
    pub positive_feedback: usize,
    pub negative_feedback: usize,
    pub total_implicit_signals: usize,
    pub signals_by_kind: std::collections::HashMap<String, usize>,
    pub at_risk_memories_count: usize,
    pub memories: Vec<MemoryDecayItem>,
    pub anti_patterns: Vec<AntiPatternItem>,
    pub feedback_events: Vec<FeedbackItem>,
    pub ebbinghaus_curve_points: Vec<CurvePoint>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryDecayItem {
    pub id: Uuid,
    pub title: String,
    pub memory_type: String,
    pub scope: String,
    pub importance: f32,
    pub confidence: f32,
    pub access_count: u32,
    pub stability_hours: f32,
    pub act_r_activation: f32,
    pub ebbinghaus_retention: f32,
    pub status: String,
    pub is_invariant: bool,
    pub is_expired: bool,
    pub last_accessed_ago_hours: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AntiPatternItem {
    pub id: String,
    pub signature: String,
    pub pattern_name: String,
    pub description: String,
    pub error_type: String,
    pub severity: String,
    pub occurrences: u64,
    pub mitigation: String,
    pub last_seen: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeedbackItem {
    pub id: Uuid,
    pub rating: String,
    pub source: String,
    pub comment: Option<String>,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CurvePoint {
    pub hour: f32,
    pub low_retention: f32,    // Importance 0.2
    pub medium_retention: f32, // Importance 0.5
    pub high_retention: f32,   // Importance 0.8
}

/// Execute cognitive observability command.
pub async fn run_observe(args: ObserveArgs, store: Arc<SqliteStore>) -> Result<()> {
    if args.json {
        let report = generate_report(&store, &args)?;
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }

    if args.live {
        run_live_tui(&store, &args).await?;
    } else {
        let report = generate_report(&store, &args)?;
        render_cli_dashboard(&report, &args);
    }

    Ok(())
}

/// Continuous live TUI refresh loop.
async fn run_live_tui(store: &SqliteStore, args: &ObserveArgs) -> Result<()> {
    // Print initial setup instructions
    println!("\x1b[2J\x1b[1;1H"); // Clear screen and home cursor
    println!("Starting Strata Cognitive Observability TUI (Ctrl+C to exit)...");

    let interval = Duration::from_secs(args.interval_secs.max(1));

    loop {
        let report = generate_report(store, args)?;

        // Clear screen with ANSI and print updated dashboard
        print!("\x1b[2J\x1b[1;1H");
        render_cli_dashboard(&report, args);
        println!(
            "\n⏳ Live auto-refresh every {}s • Press Ctrl+C to stop",
            args.interval_secs
        );

        tokio::time::sleep(interval).await;
    }
}

/// Gather and compute all cognitive metrics from SQLite store.
pub fn generate_report(store: &SqliteStore, args: &ObserveArgs) -> Result<CognitiveReport> {
    let now = Utc::now();
    let calculator = DecayCalculator::with_default_config();

    // 1. Gather semantic facts
    let facts = store
        .get_all_semantic_facts(None, None, 10000)
        .unwrap_or_default();
    let mut active_facts = 0;
    let mut deprecated_facts = 0;
    let mut memory_items = Vec::new();

    for f in &facts {
        if f.status == FactStatus::Active {
            active_facts += 1;
        } else {
            deprecated_facts += 1;
        }

        let access_logs = store.get_memory_access_logs(&f.id).unwrap_or_default();
        let metrics = calculator.evaluate_semantic_fact(f, &access_logs, now);

        let elapsed_last_access = if access_logs.is_empty() {
            (now - f.created_at).num_seconds().max(0) as f32 / 3600.0
        } else {
            access_logs
                .iter()
                .map(|t| (now - *t).num_seconds().max(0) as f32 / 3600.0)
                .fold(f32::INFINITY, f32::min)
        };

        let status_str = if f.importance >= calculator.config.invariant_threshold {
            "Invariant".to_string()
        } else if metrics.retention >= 0.5 {
            "Healthy".to_string()
        } else if metrics.retention >= calculator.config.prune_threshold {
            "At Risk".to_string()
        } else {
            "Expired".to_string()
        };

        memory_items.push(MemoryDecayItem {
            id: f.id,
            title: f.statement.clone(),
            memory_type: "SemanticFact".to_string(),
            scope: f.scope.to_string(),
            importance: f.importance,
            confidence: f.confidence,
            access_count: access_logs.len() as u32,
            stability_hours: metrics.stability,
            act_r_activation: metrics.activation,
            ebbinghaus_retention: metrics.retention,
            status: status_str,
            is_invariant: f.importance >= calculator.config.invariant_threshold,
            is_expired: metrics.is_expired,
            last_accessed_ago_hours: elapsed_last_access,
        });
    }

    // 2. Gather general memories
    let general_mems = store
        .get_all_memories(None, None, 10000)
        .unwrap_or_default();
    for m in &general_mems {
        let access_logs = store.get_memory_access_logs(&m.id).unwrap_or_default();
        let metrics = calculator.evaluate_memory_record(m, &access_logs, now);

        let elapsed_last_access = if access_logs.is_empty() {
            (now - m.created_at).num_seconds().max(0) as f32 / 3600.0
        } else {
            access_logs
                .iter()
                .map(|t| (now - *t).num_seconds().max(0) as f32 / 3600.0)
                .fold(f32::INFINITY, f32::min)
        };

        let status_str = if m.importance >= calculator.config.invariant_threshold {
            "Invariant".to_string()
        } else if metrics.retention >= 0.5 {
            "Healthy".to_string()
        } else if metrics.retention >= calculator.config.prune_threshold {
            "At Risk".to_string()
        } else {
            "Expired".to_string()
        };

        memory_items.push(MemoryDecayItem {
            id: m.id,
            title: m
                .summary
                .clone()
                .unwrap_or_else(|| m.content.chars().take(60).collect::<String>()),
            memory_type: m.memory_type.to_string(),
            scope: m.scope.to_string(),
            importance: m.importance,
            confidence: m.confidence,
            access_count: access_logs.len() as u32,
            stability_hours: metrics.stability,
            act_r_activation: metrics.activation,
            ebbinghaus_retention: metrics.retention,
            status: status_str,
            is_invariant: m.importance >= calculator.config.invariant_threshold,
            is_expired: metrics.is_expired,
            last_accessed_ago_hours: elapsed_last_access,
        });
    }

    // Filter scope if specified
    if let Some(ref sc) = args.scope {
        memory_items.retain(|m| m.scope.contains(sc));
    }

    // Sort by retention (lowest retention first for risk inspection)
    memory_items.sort_by(|a, b| {
        a.ebbinghaus_retention
            .partial_cmp(&b.ebbinghaus_retention)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let at_risk_count = memory_items
        .iter()
        .filter(|m| !m.is_invariant && m.ebbinghaus_retention < 0.5)
        .count();

    // 3. Gather Procedural Skills
    let skills = store
        .get_all_procedural_skills(None, 1000)
        .unwrap_or_default();

    // 4. Gather Failure Patterns / Anti-Patterns
    let patterns = store.search_failures(None, None, 1000).unwrap_or_default();
    let mut anti_pattern_items = Vec::new();
    for p in patterns {
        let sev_str = match p.severity {
            FailureSeverity::Critical => "CRITICAL",
            FailureSeverity::High => "HIGH",
            FailureSeverity::Medium => "MEDIUM",
            FailureSeverity::Low => "LOW",
        };
        anti_pattern_items.push(AntiPatternItem {
            id: p.id.to_string(),
            signature: p.signature,
            pattern_name: p.pattern_name,
            description: p.description,
            error_type: p.error_type,
            severity: sev_str.to_string(),
            occurrences: p.occurrences,
            mitigation: p.mitigation,
            last_seen: p.last_seen.to_rfc3339(),
        });
    }

    // 5. Gather Feedback & Signals
    let feedback_events = store.get_feedback_events(None).unwrap_or_default();
    let mut pos_count = 0;
    let mut neg_count = 0;
    let mut feedback_items = Vec::new();

    for fb in &feedback_events {
        if fb.rating == FeedbackRating::Positive {
            pos_count += 1;
        } else {
            neg_count += 1;
        }
        feedback_items.push(FeedbackItem {
            id: fb.id,
            rating: fb.rating.to_string(),
            source: fb.source.clone(),
            comment: fb.comment.clone(),
            timestamp: fb.timestamp,
        });
    }

    let implicit_signals = store.get_implicit_signals(None).unwrap_or_default();
    let mut signals_by_kind = std::collections::HashMap::new();
    for sig in &implicit_signals {
        *signals_by_kind.entry(sig.kind.to_string()).or_insert(0) += 1;
    }

    // 6. Generate Ebbinghaus curve sample points (0 to horizon_hours)
    let horizon = args.horizon_hours.max(24.0);
    let steps = 16;
    let mut curve_points = Vec::new();
    let stab_low = calculator.calculate_stability(0, 0.2);
    let stab_med = calculator.calculate_stability(3, 0.5);
    let stab_high = calculator.calculate_stability(10, 0.8);

    for i in 0..=steps {
        let h = (horizon * (i as f32) / (steps as f32)).round();
        let ret_low = calculator.calculate_ebbinghaus_retention(h, stab_low);
        let ret_med = calculator.calculate_ebbinghaus_retention(h, stab_med);
        let ret_high = calculator.calculate_ebbinghaus_retention(h, stab_high);

        curve_points.push(CurvePoint {
            hour: h,
            low_retention: ret_low,
            medium_retention: ret_med,
            high_retention: ret_high,
        });
    }

    Ok(CognitiveReport {
        generated_at: now,
        total_memories: memory_items.len(),
        active_semantic_facts: active_facts,
        deprecated_semantic_facts: deprecated_facts,
        procedural_skills: skills.len(),
        anti_patterns_count: anti_pattern_items.len(),
        total_feedback_events: feedback_events.len(),
        positive_feedback: pos_count,
        negative_feedback: neg_count,
        total_implicit_signals: implicit_signals.len(),
        signals_by_kind,
        at_risk_memories_count: at_risk_count,
        memories: memory_items,
        anti_patterns: anti_pattern_items,
        feedback_events: feedback_items,
        ebbinghaus_curve_points: curve_points,
    })
}

/// Render rich terminal UI with ASCII Ebbinghaus graphs and tables.
pub fn render_cli_dashboard(report: &CognitiveReport, args: &ObserveArgs) {
    let focus = args.tab.as_deref().unwrap_or("overview").to_lowercase();

    // 1. Header Banner
    println!("\x1b[1;36m╔══════════════════════════════════════════════════════════════════════════════════════════╗\x1b[0m");
    println!("\x1b[1;36m║\x1b[0m \x1b[1;37m🧠 STRATA COGNITIVE OBSERVABILITY & DECAY DASHBOARD\x1b[0m                                \x1b[1;36m║\x1b[0m");
    println!(
        "\x1b[1;36m║\x1b[0m \x1b[90mActive Snapshot: {} • Scope: {}\x1b[0m \x1b[1;36m║\x1b[0m",
        report.generated_at.format("%Y-%m-%d %H:%M:%S UTC"),
        args.scope.as_deref().unwrap_or("[All Global/Project]")
    );
    println!("\x1b[1;36m╚══════════════════════════════════════════════════════════════════════════════════════════╝\x1b[0m");

    // 2. Telemetry Summary Cards
    println!("\n\x1b[1;33m┌── Overview Telemetry ──────────────────────────────────────────────────────────────────┐\x1b[0m");
    println!("│ Total Stored Memories: \x1b[1;32m{:<5}\x1b[0m │ Active JTMS Facts: \x1b[1;32m{:<5}\x1b[0m │ Procedural Skills: \x1b[1;32m{:<5}\x1b[0m │",
        report.total_memories, report.active_semantic_facts, report.procedural_skills);
    println!("│ Mined Anti-Patterns:   \x1b[1;31m{:<5}\x1b[0m │ Memories At Risk:  \x1b[1;33m{:<5}\x1b[0m │ Deprecated Facts:  \x1b[1;90m{:<5}\x1b[0m │",
        report.anti_patterns_count, report.at_risk_memories_count, report.deprecated_semantic_facts);

    let fb_ratio = if report.total_feedback_events > 0 {
        (report.positive_feedback as f32 / report.total_feedback_events as f32) * 100.0
    } else {
        100.0
    };
    println!("│ Feedback Approval:     \x1b[1;32m{:>4.1}%\x1b[0m │ Implicit Signals:  \x1b[1;36m{:<5}\x1b[0m │ Feedback Events:   \x1b[1;37m{:<5}\x1b[0m │",
        fb_ratio, report.total_implicit_signals, report.total_feedback_events);
    println!("\x1b[1;33m└────────────────────────────────────────────────────────────────────────────────────────┘\x1b[0m");

    // 3. Render Ebbinghaus Mathematical Decay Curve (if overview or decay)
    if focus == "overview" || focus == "decay" {
        println!(
            "\n\x1b[1;35m📈 Mathematical Ebbinghaus Retention Curves R(t) = exp(-t / S_m)\x1b[0m"
        );
        println!(
            "\x1b[90m   Simulated across time horizon: 0h → {:.0}h\x1b[0m",
            args.horizon_hours
        );
        println!("   \x1b[32m■\x1b[0m High Importance (I=0.8, 10 Accesses)  \x1b[33m▲\x1b[0m Medium (I=0.5, 3 Acc)  \x1b[31m▼\x1b[0m Low (I=0.2, 0 Acc)");
        println!();
        print_ascii_ebbinghaus_graph(&report.ebbinghaus_curve_points);
    }

    // 4. Memory Retention & ACT-R Activation Inspector Table
    if focus == "overview" || focus == "decay" {
        println!(
            "\n\x1b[1;34m🔍 Memory Decay & ACT-R Activation Inspector (Sorted by Risk):\x1b[0m"
        );
        println!("┌────────────┬─────────────────────────────┬──────────┬──────────┬──────────┬──────────┬────────────┐");
        println!("│ Status     │ Content / Summary           │ Scope    │ ACT-R Aₘ │ Stab(hr) │ Retent % │ Last Access│");
        println!("├────────────┼─────────────────────────────┼──────────┼──────────┼──────────┼──────────┼────────────┤");

        let display_limit = args.limit.min(report.memories.len());
        if display_limit == 0 {
            println!("│ (No memories found matching criteria)                                                 │");
        } else {
            for m in report.memories.iter().take(display_limit) {
                let status_colored = match m.status.as_str() {
                    "Invariant" => "\x1b[1;35mINVARIANT \x1b[0m",
                    "Healthy" => "\x1b[1;32mHEALTHY   \x1b[0m",
                    "At Risk" => "\x1b[1;33mAT RISK   \x1b[0m",
                    _ => "\x1b[1;31mEXPIRED   \x1b[0m",
                };

                let short_title: String = m.title.chars().take(27).collect();
                let short_scope: String = m.scope.chars().take(8).collect();
                let ret_pct = m.ebbinghaus_retention * 100.0;

                println!(
                    "│ {} │ {:<27} │ {:<8} │ {:>8.2} │ {:>8.1} │ {:>7.1}% │ {:>8.1}h │",
                    status_colored,
                    short_title,
                    short_scope,
                    m.act_r_activation,
                    m.stability_hours,
                    ret_pct,
                    m.last_accessed_ago_hours
                );
            }
        }
        println!("└────────────┴─────────────────────────────┴──────────┴──────────┴──────────┴──────────┴────────────┘");
    }

    // 5. Anti-Pattern & Failure Pattern Audit Panel
    if focus == "overview" || focus == "anti-patterns" || focus == "failures" {
        println!("\n\x1b[1;31m🛡️ Captured Anti-Patterns & Verified Failure Mitigations:\x1b[0m");
        if report.anti_patterns.is_empty() {
            println!("   \x1b[90m✓ No anti-patterns currently active (all failure patterns clean).\x1b[0m");
        } else {
            println!("┌──────────┬─────────────────────────────────┬──────┬────────────────────────────────────────┐");
            println!("│ Severity │ Pattern Name / Signature        │ Occ  │ Proven Mitigation Rule                 │");
            println!("├──────────┼─────────────────────────────────┼──────┼────────────────────────────────────────┤");
            for ap in report.anti_patterns.iter().take(args.limit) {
                let sev_colored = match ap.severity.as_str() {
                    "CRITICAL" => "\x1b[1;41;37m CRITICAL \x1b[0m",
                    "HIGH" => "\x1b[1;31m   HIGH   \x1b[0m",
                    "MEDIUM" => "\x1b[1;33m  MEDIUM  \x1b[0m",
                    _ => "\x1b[90m   LOW    \x1b[0m",
                };
                let name: String = ap.pattern_name.chars().take(31).collect();
                let mit: String = ap.mitigation.chars().take(38).collect();

                println!(
                    "│ {} │ {:<31} │ {:>4} │ {:<38} │",
                    sev_colored, name, ap.occurrences, mit
                );
            }
            println!("└──────────┴─────────────────────────────────┴──────┴────────────────────────────────────────┘");
        }
    }

    // 6. Feedback & Reinforcement Alignment Signals Panel
    if focus == "overview" || focus == "feedback" || focus == "signals" {
        println!("\n\x1b[1;32m🎯 Reinforcement Feedback & Implicit Behavioural Signals:\x1b[0m");
        println!(
            "   • Explicit Ratings: \x1b[32m{} Positive 👍\x1b[0m | \x1b[31m{} Negative 👎\x1b[0m",
            report.positive_feedback, report.negative_feedback
        );

        if !report.signals_by_kind.is_empty() {
            print!("   • Implicit Signals: ");
            for (k, v) in &report.signals_by_kind {
                print!("\x1b[36m{} (x{})\x1b[0m  ", k, v);
            }
            println!();
        }

        if !report.feedback_events.is_empty() {
            println!("\n   Recent Feedback Log:");
            for fb in report.feedback_events.iter().take(4) {
                let icon = if fb.rating == "positive" {
                    "\x1b[32m[+]\x1b[0m"
                } else {
                    "\x1b[31m[-]\x1b[0m"
                };
                println!(
                    "     {} {} \x1b[90m(from {})\x1b[0m: {}",
                    icon,
                    fb.timestamp.format("%H:%M:%S"),
                    fb.source,
                    fb.comment.as_deref().unwrap_or("(No textual comment)")
                );
            }
        }
    }

    println!();
}

/// Helper function to draw an ASCII 2D Ebbinghaus retention curve graph.
fn print_ascii_ebbinghaus_graph(points: &[CurvePoint]) {
    if points.is_empty() {
        return;
    }

    let y_levels: [f32; 7] = [1.0, 0.8, 0.6, 0.4, 0.2, 0.05, 0.0];

    for &thresh in &y_levels {
        let label = if (thresh * 100.0f32).round() == 5.0 {
            "\x1b[31mPRUNE(5%)\x1b[0m".to_string()
        } else {
            format!("{:>3.0}%", thresh * 100.0)
        };

        print!("  {:>10} │ ", label);

        for p in points {
            let symbol = if (p.high_retention - thresh).abs() < 0.08 {
                "\x1b[32m■\x1b[0m"
            } else if (p.medium_retention - thresh).abs() < 0.08 {
                "\x1b[33m▲\x1b[0m"
            } else if (p.low_retention - thresh).abs() < 0.08 {
                "\x1b[31m▼\x1b[0m"
            } else if thresh == 0.05 {
                "\x1b[90m┄\x1b[0m" // Pruning boundary line
            } else {
                " "
            };

            print!(" {:<2}", symbol);
        }
        println!();
    }

    // Draw X-axis
    let x_len = points.len() * 3 + 2;
    println!("             └{}", "─".repeat(x_len));
    print!("              ");
    for p in points.iter().step_by(2) {
        print!("{:<6.0}h", p.hour);
    }
    println!();
}
