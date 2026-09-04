use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Gauge, Paragraph},
    Frame,
};

use crate::tui::app::App;
use crate::tui::theme::Theme;

pub fn render(f: &mut Frame, app: &App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(7), // KPI Cards
            Constraint::Length(4), // Retention Health Gauge
            Constraint::Min(8),    // Split: Cognitive Status & Recent Activity
        ])
        .split(area);

    // 1. Render Top KPI Cards (4 columns)
    let kpi_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(25),
            Constraint::Percentage(25),
            Constraint::Percentage(25),
            Constraint::Percentage(25),
        ])
        .split(chunks[0]);

    // KPI 1: Memory Health
    let healthy_count = app.report.memories.iter().filter(|m| m.status == "Healthy" || m.status == "Invariant").count();
    let health_pct = if app.report.total_memories > 0 {
        (healthy_count as f64 / app.report.total_memories as f64) * 100.0
    } else {
        100.0
    };
    render_card(
        f,
        kpi_chunks[0],
        "🧠 COGNITIVE HEALTH",
        &format!("{:.1}%", health_pct),
        &format!("{} / {} Healthy", healthy_count, app.report.total_memories),
        Theme::SUCCESS,
    );

    // KPI 2: Total Memories & Tiers
    let core_invariants = app.report.memories.iter().filter(|m| m.is_invariant).count();
    render_card(
        f,
        kpi_chunks[1],
        "📦 PERSISTENT MEMORY",
        &format!("{}", app.report.total_memories),
        &format!("{} Core Tier (HITL Locked)", core_invariants),
        Theme::PRIMARY,
    );

    // KPI 3: Anti-Patterns Defense
    render_card(
        f,
        kpi_chunks[2],
        "🛡️ ANTI-PATTERNS",
        &format!("{}", app.report.anti_patterns_count),
        "Preemptive Guardrails Active",
        if app.report.anti_patterns_count > 0 { Theme::WARNING } else { Theme::SUCCESS },
    );

    // KPI 4: Feedback & Signals
    render_card(
        f,
        kpi_chunks[3],
        "⚡ ADAPTIVE SIGNALS",
        &format!("{}", app.report.total_feedback_events + app.report.total_implicit_signals),
        &format!("{} Pos / {} Neg", app.report.positive_feedback, app.report.negative_feedback),
        Theme::ACCENT,
    );

    // 2. Render Retention Health Gauge
    let gauge_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Theme::border_style())
        .border_type(BorderType::Rounded)
        .title(" EBBINGHAUS RETENTION SPECTRUM [R(t) = exp(-t / S_m)] ");

    let gauge = Gauge::default()
        .block(gauge_block)
        .gauge_style(Style::default().fg(Theme::SUCCESS).bg(Theme::CARD_BG))
        .percent(health_pct.round() as u16)
        .label(format!("Cognitive Durability: {:.1}% Active Retention", health_pct));
    f.render_widget(gauge, chunks[1]);

    // 3. Render Bottom Split: Status & Details
    let bottom_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(chunks[2]);

    // Left: Architectural Invariants
    let mut status_lines = vec![
        Line::from(vec![
            Span::styled("● Storage Engine: ", Style::default().fg(Theme::TEXT_MUTED)),
            Span::styled("SQLite 3 (WAL Mode + FTS5 BM25)", Style::default().fg(Theme::TEXT).add_modifier(Modifier::BOLD)),
        ]),
        Line::from(vec![
            Span::styled("● Local Embeddings: ", Style::default().fg(Theme::TEXT_MUTED)),
            Span::styled("FastEmbed ONNX (All-MiniLM-L6-v2, 384-dim)", Style::default().fg(Theme::SECONDARY)),
        ]),
        Line::from(vec![
            Span::styled("● Code Grounding: ", Style::default().fg(Theme::TEXT_MUTED)),
            Span::styled("Tree-Sitter AST & Git Merkle Hash Trees", Style::default().fg(Theme::SUCCESS)),
        ]),
        Line::from(vec![
            Span::styled("● Truth Maintenance: ", Style::default().fg(Theme::TEXT_MUTED)),
            Span::styled("JTMS v2 Deterministic Propositional Revision", Style::default().fg(Theme::PRIMARY)),
        ]),
        Line::from(vec![
            Span::styled("● Privacy Level: ", Style::default().fg(Theme::TEXT_MUTED)),
            Span::styled("100% Local-First / Zero-Telemetry", Style::default().fg(Theme::SUCCESS).add_modifier(Modifier::BOLD)),
        ]),
    ];

    if let Some(ref msg) = app.status_message {
        status_lines.push(Line::from(""));
        status_lines.push(Line::from(vec![
            Span::styled("⚡ Status: ", Style::default().fg(Theme::WARNING)),
            Span::styled(msg, Style::default().fg(Theme::TEXT)),
        ]));
    }

    let status_block = Paragraph::new(status_lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Theme::border_style())
                .border_type(BorderType::Rounded)
                .title(" ARCHITECTURAL INVARIANTS "),
        );
    f.render_widget(status_block, bottom_chunks[0]);

    // Right: Mathematical Memory Curves Simulation
    let mut curve_lines = vec![
        Line::from(Span::styled("Simulated Decay Over Time Horizon (0h → 168h):", Style::default().fg(Theme::TEXT_MUTED))),
        Line::from(""),
    ];

    for pt in app.report.ebbinghaus_curve_points.iter().take(6) {
        let ret = pt.medium_retention;
        let bar_len = (ret * 30.0) as usize;
        let bar_str = "█".repeat(bar_len);
        let color = if ret >= 0.7 {
            Theme::SUCCESS
        } else if ret >= 0.4 {
            Theme::WARNING
        } else {
            Theme::DANGER
        };

        curve_lines.push(Line::from(vec![
            Span::styled(format!("{:>5.0}h: ", pt.hour), Style::default().fg(Theme::TEXT_MUTED)),
            Span::styled(bar_str, Style::default().fg(color)),
            Span::styled(format!(" {:>4.1}%", ret * 100.0), Style::default().fg(Theme::TEXT)),
        ]));
    }

    let curve_block = Paragraph::new(curve_lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Theme::border_style())
                .border_type(BorderType::Rounded)
                .title(" MATHEMATICAL RETENTION PROJECTION "),
        );
    f.render_widget(curve_block, bottom_chunks[1]);
}

fn render_card(f: &mut Frame, area: Rect, title: &str, value: &str, subtitle: &str, color: Color) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(color))
        .border_type(BorderType::Rounded)
        .title(Span::styled(format!(" {} ", title), Style::default().fg(color).add_modifier(Modifier::BOLD)));

    let text = vec![
        Line::from(""),
        Line::from(Span::styled(
            value,
            Style::default().fg(Theme::TEXT).add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(subtitle, Style::default().fg(Theme::TEXT_MUTED))),
    ];

    let p = Paragraph::new(text).block(block).alignment(Alignment::Center);
    f.render_widget(p, area);
}
