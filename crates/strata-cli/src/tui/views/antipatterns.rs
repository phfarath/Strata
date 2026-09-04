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
        .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
        .split(area);

    // 1. Anti-Patterns Table (Left)
    let header_cells = ["Severity", "Pattern Name", "Type", "Hits"]
        .iter()
        .map(|h| Cell::from(*h).style(Style::default().fg(Theme::PRIMARY).add_modifier(Modifier::BOLD)));
    let header = Row::new(header_cells).height(1).bottom_margin(1);

    let rows = app.report.anti_patterns.iter().enumerate().map(|(idx, p)| {
        let is_selected = idx == app.selected_antipattern_idx;
        let style = if is_selected {
            Theme::selected_row_style()
        } else {
            Style::default().fg(Theme::TEXT)
        };

        let sev_style = Theme::status_style(&p.severity);
        let name_short: String = p.pattern_name.chars().take(28).collect();
        let type_short: String = p.error_type.chars().take(12).collect();

        Row::new(vec![
            Cell::from(p.severity.clone()).style(sev_style),
            Cell::from(name_short),
            Cell::from(type_short),
            Cell::from(format!("{}", p.occurrences)),
        ])
        .style(style)
        .height(1)
    });

    let table_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Theme::border_style())
        .border_type(BorderType::Rounded)
        .title(format!(
            " CAPTURED ANTI-PATTERNS ({}) [Use ↑/↓ or j/k to navigate] ",
            app.report.anti_patterns.len()
        ));

    let table = Table::new(
        rows,
        [
            Constraint::Length(12),
            Constraint::Min(25),
            Constraint::Length(15),
            Constraint::Length(6),
        ],
    )
    .header(header)
    .block(table_block);

    f.render_widget(table, chunks[0]);

    // 2. Mitigation Inspector (Right)
    let selected_pattern = app.report.anti_patterns.get(app.selected_antipattern_idx);

    let detail_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Theme::focused_border_style())
        .border_type(BorderType::Rounded)
        .title(" FAILURE DEFENSE RADAR ");

    if let Some(p) = selected_pattern {
        let sev_style = Theme::status_style(&p.severity);

        let details = vec![
            Line::from(vec![
                Span::styled("Pattern:    ", Style::default().fg(Theme::TEXT_MUTED)),
                Span::styled(&p.pattern_name, Style::default().fg(Theme::TEXT).add_modifier(Modifier::BOLD)),
            ]),
            Line::from(vec![
                Span::styled("Severity:   ", Style::default().fg(Theme::TEXT_MUTED)),
                Span::styled(&p.severity, sev_style),
                Span::styled(format!("  (Occurred {} times)", p.occurrences), Style::default().fg(Theme::TEXT_MUTED)),
            ]),
            Line::from(vec![
                Span::styled("Error Type: ", Style::default().fg(Theme::TEXT_MUTED)),
                Span::styled(&p.error_type, Style::default().fg(Theme::SECONDARY)),
            ]),
            Line::from(""),
            Line::from(Span::styled("── MITIGATION RULE (AGENT PROMPT INJECTED) ──", Style::default().fg(Theme::TEXT_MUTED))),
            Line::from(Span::styled(&p.mitigation, Style::default().fg(Theme::SUCCESS).add_modifier(Modifier::BOLD))),
            Line::from(""),
            Line::from(Span::styled("── ERROR SIGNATURE / REGEX ──", Style::default().fg(Theme::TEXT_MUTED))),
            Line::from(Span::styled(&p.signature, Style::default().fg(Theme::DANGER))),
            Line::from(""),
            Line::from(Span::styled("── DESCRIPTION & IMPACT ──", Style::default().fg(Theme::TEXT_MUTED))),
            Line::from(Span::styled(&p.description, Style::default().fg(Theme::TEXT))),
        ];

        let p_widget = Paragraph::new(details)
            .block(detail_block)
            .wrap(Wrap { trim: true });
        f.render_widget(p_widget, chunks[1]);
    } else {
        let p_widget = Paragraph::new("No anti-patterns currently active in repository.\nAll compiler and command executions are clean.")
            .block(detail_block)
            .style(Style::default().fg(Theme::TEXT_MUTED));
        f.render_widget(p_widget, chunks[1]);
    }
}
