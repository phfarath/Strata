use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Cell, Paragraph, Row, Table, Wrap},
    Frame,
};

use crate::tui::app::App;
use crate::tui::theme::Theme;

pub fn render(f: &mut Frame, app: &App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
        .split(area);

    // 1. Memories Table (Left)
    let header_cells = ["Status", "Title / Content", "Scope", "ACT-R Aₘ", "Retent %"]
        .iter()
        .map(|h| Cell::from(*h).style(Style::default().fg(Theme::PRIMARY).add_modifier(Modifier::BOLD)));
    let header = Row::new(header_cells).height(1).bottom_margin(1);

    let rows = app.report.memories.iter().enumerate().map(|(idx, m)| {
        let is_selected = idx == app.selected_memory_idx;
        let style = if is_selected {
            Theme::selected_row_style()
        } else {
            Style::default().fg(Theme::TEXT)
        };

        let status_style = Theme::status_style(&m.status);
        let ret_pct = m.ebbinghaus_retention * 100.0;
        let title_short: String = m.title.chars().take(30).collect();

        Row::new(vec![
            Cell::from(m.status.clone()).style(status_style),
            Cell::from(title_short),
            Cell::from(m.scope.clone()),
            Cell::from(format!("{:.2}", m.act_r_activation)),
            Cell::from(format!("{:.1}%", ret_pct)),
        ])
        .style(style)
        .height(1)
    });

    let table_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Theme::border_style())
        .border_type(BorderType::Rounded)
        .title(format!(
            " PERSISTENT MEMORIES ({}) [Use ↑/↓ or j/k to navigate] ",
            app.report.memories.len()
        ));

    let table = Table::new(
        rows,
        [
            Constraint::Length(12),
            Constraint::Min(25),
            Constraint::Length(10),
            Constraint::Length(10),
            Constraint::Length(10),
        ],
    )
    .header(header)
    .block(table_block);

    f.render_widget(table, chunks[0]);

    // 2. Selected Memory Inspector (Right)
    let selected_memory = app.report.memories.get(app.selected_memory_idx);

    let detail_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Theme::focused_border_style())
        .border_type(BorderType::Rounded)
        .title(" COGNITIVE INSPECTOR ");

    if let Some(m) = selected_memory {
        let status_style = Theme::status_style(&m.status);
        let ret_pct = m.ebbinghaus_retention * 100.0;

        let details = vec![
            Line::from(vec![
                Span::styled("Memory ID: ", Style::default().fg(Theme::TEXT_MUTED)),
                Span::styled(m.id.to_string(), Style::default().fg(Theme::TEXT).add_modifier(Modifier::BOLD)),
            ]),
            Line::from(vec![
                Span::styled("Status:    ", Style::default().fg(Theme::TEXT_MUTED)),
                Span::styled(&m.status, status_style),
                Span::styled(if m.is_invariant { " (Core Tier Invariant)" } else { "" }, Style::default().fg(Theme::ACCENT)),
            ]),
            Line::from(vec![
                Span::styled("Scope:     ", Style::default().fg(Theme::TEXT_MUTED)),
                Span::styled(&m.scope, Style::default().fg(Theme::SECONDARY)),
            ]),
            Line::from(""),
            Line::from(Span::styled("── MATHEMATICAL METRICS ──", Style::default().fg(Theme::TEXT_MUTED))),
            Line::from(vec![
                Span::styled("ACT-R Activation (Aₘ): ", Style::default().fg(Theme::TEXT_MUTED)),
                Span::styled(format!("{:.4}", m.act_r_activation), Style::default().fg(Theme::PRIMARY).add_modifier(Modifier::BOLD)),
            ]),
            Line::from(vec![
                Span::styled("Stability Half-Life:   ", Style::default().fg(Theme::TEXT_MUTED)),
                Span::styled(format!("{:.1} hours", m.stability_hours), Style::default().fg(Theme::TEXT)),
            ]),
            Line::from(vec![
                Span::styled("Current Retention:     ", Style::default().fg(Theme::TEXT_MUTED)),
                Span::styled(format!("{:.2}%", ret_pct), status_style),
            ]),
            Line::from(vec![
                Span::styled("Last Accessed:         ", Style::default().fg(Theme::TEXT_MUTED)),
                Span::styled(format!("{:.1} hours ago", m.last_accessed_ago_hours), Style::default().fg(Theme::TEXT)),
            ]),
            Line::from(""),
            Line::from(Span::styled("── SUMMARY / CONTENT ──", Style::default().fg(Theme::TEXT_MUTED))),
            Line::from(Span::styled(&m.title, Style::default().fg(Theme::TEXT))),
            Line::from(""),
            Line::from(Span::styled(
                "Equation: R(t) = exp(-t / S_m)",
                Style::default().fg(Theme::TEXT_MUTED).add_modifier(Modifier::ITALIC),
            )),
        ];

        let p = Paragraph::new(details)
            .block(detail_block)
            .wrap(Wrap { trim: true });
        f.render_widget(p, chunks[1]);
    } else {
        let p = Paragraph::new("No memory record selected.")
            .block(detail_block)
            .style(Style::default().fg(Theme::TEXT_MUTED));
        f.render_widget(p, chunks[1]);
    }
}
