use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Cell, Gauge, Paragraph, Row, Table, Wrap},
    Frame,
};

use crate::tui::app::{App, DashboardItem};
use crate::tui::theme::Theme;

pub fn render(f: &mut Frame, app: &App) {
    let size = f.area();

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Header
            Constraint::Length(4), // KPIs + Retention Bar
            Constraint::Min(8),    // 50/50 Split: Items | Inspector
            Constraint::Length(1), // Minimal Footer
        ])
        .split(size);

    render_header(f, app, chunks[0]);
    render_kpis(f, app, chunks[1]);
    render_split(f, app, chunks[2]);
    render_footer(f, app, chunks[3]);
}

fn render_header(f: &mut Frame, app: &App, area: Rect) {
    let header_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(70), Constraint::Percentage(30)])
        .split(area);

    let title_line = Line::from(vec![
        Span::styled("STRATA", Style::default().fg(Theme::PRIMARY).add_modifier(Modifier::BOLD)),
        Span::styled(" │ ", Style::default().fg(Theme::BORDER)),
        Span::styled(&app.workspace_name, Style::default().fg(Theme::TEXT)),
    ]);

    let title_p = Paragraph::new(title_line).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Theme::border_style())
            .border_type(BorderType::Rounded),
    );
    f.render_widget(title_p, header_chunks[0]);

    let status_line = Line::from(vec![
        Span::styled("● ", Style::default().fg(Theme::SUCCESS)),
        Span::styled("Local", Style::default().fg(Theme::SUCCESS).add_modifier(Modifier::BOLD)),
    ]);

    let status_p = Paragraph::new(status_line)
        .alignment(Alignment::Right)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Theme::border_style())
                .border_type(BorderType::Rounded),
        );
    f.render_widget(status_p, header_chunks[1]);
}

fn render_kpis(f: &mut Frame, app: &App, area: Rect) {
    let kpi_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(2), Constraint::Length(2)])
        .split(area);

    let cards = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(25),
            Constraint::Percentage(25),
            Constraint::Percentage(25),
            Constraint::Percentage(25),
        ])
        .split(kpi_chunks[0]);

    let total_mems = app.report.total_memories;
    let invariants = app.report.memories.iter().filter(|m| m.is_invariant).count();
    let guardrails = app.report.anti_patterns.len();
    
    let active_mems = app.report.memories.iter().filter(|m| !m.is_expired).count();
    let avg_retention = if total_mems > 0 {
        (active_mems as f32 / total_mems as f32) * 100.0
    } else {
        100.0
    };

    let p1 = Paragraph::new(Line::from(vec![
        Span::styled("Memórias: ", Style::default().fg(Theme::TEXT_MUTED)),
        Span::styled(format!("{}", total_mems), Style::default().fg(Theme::TEXT).add_modifier(Modifier::BOLD)),
    ]));
    let p2 = Paragraph::new(Line::from(vec![
        Span::styled("Core: ", Style::default().fg(Theme::TEXT_MUTED)),
        Span::styled(format!("{}", invariants), Style::default().fg(Theme::SUCCESS).add_modifier(Modifier::BOLD)),
    ]));
    let p3 = Paragraph::new(Line::from(vec![
        Span::styled("Guardrails: ", Style::default().fg(Theme::TEXT_MUTED)),
        Span::styled(format!("{}", guardrails), Style::default().fg(Theme::DANGER).add_modifier(Modifier::BOLD)),
    ]));
    let p4 = Paragraph::new(Line::from(vec![
        Span::styled("Retenção: ", Style::default().fg(Theme::TEXT_MUTED)),
        Span::styled(format!("{:.0}%", avg_retention), Style::default().fg(Theme::PRIMARY).add_modifier(Modifier::BOLD)),
    ]));

    f.render_widget(p1, cards[0]);
    f.render_widget(p2, cards[1]);
    f.render_widget(p3, cards[2]);
    f.render_widget(p4, cards[3]);

    let gauge_ratio = (avg_retention / 100.0).clamp(0.0, 1.0);
    let gauge = Gauge::default()
        .gauge_style(Style::default().fg(Theme::SUCCESS).bg(Color::Rgb(30, 41, 59)))
        .ratio(gauge_ratio as f64)
        .label(format!("Saúde de Retenção: {:.1}%", avg_retention));
    f.render_widget(gauge, kpi_chunks[1]);
}

fn render_split(f: &mut Frame, app: &App, area: Rect) {
    let split_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);

    // Left List
    let header_cells = ["Status", "Item", "Retenção"]
        .iter()
        .map(|h| Cell::from(*h).style(Style::default().fg(Theme::PRIMARY).add_modifier(Modifier::BOLD)));
    let header = Row::new(header_cells).height(1).bottom_margin(1);

    let total = app.total_items();
    let rows = (0..total).map(|idx| {
        let is_selected = idx == app.selected_idx;
        let row_style = if is_selected {
            Theme::selected_row_style()
        } else {
            Style::default().fg(Theme::TEXT)
        };

        match app.get_item(idx) {
            Some(DashboardItem::Memory(m)) => {
                let (status_tag, status_style) = if m.is_invariant {
                    ("[CORE]", Style::default().fg(Theme::SUCCESS).add_modifier(Modifier::BOLD))
                } else if m.is_expired {
                    ("[EXPR]", Style::default().fg(Theme::TEXT_MUTED))
                } else {
                    ("[ACTV]", Style::default().fg(Theme::SECONDARY))
                };

                let title_short: String = m.title.chars().take(28).collect();
                let ret_str = if m.is_invariant {
                    "100%".to_string()
                } else {
                    format!("{:.0}%", m.ebbinghaus_retention * 100.0)
                };

                Row::new(vec![
                    Cell::from(status_tag).style(status_style),
                    Cell::from(title_short),
                    Cell::from(ret_str),
                ])
                .style(row_style)
                .height(1)
            }
            Some(DashboardItem::AntiPattern(ap)) => {
                let status_tag = "[GUARD]";
                let status_style = Style::default().fg(Theme::DANGER).add_modifier(Modifier::BOLD);
                let title_short: String = ap.pattern_name.chars().take(28).collect();

                Row::new(vec![
                    Cell::from(status_tag).style(status_style),
                    Cell::from(title_short),
                    Cell::from("Ativo".to_string()).style(Style::default().fg(Theme::DANGER)),
                ])
                .style(row_style)
                .height(1)
            }
            None => Row::new(vec![Cell::from("-"), Cell::from("-"), Cell::from("-")]),
        }
    });

    let list_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Theme::focused_border_style())
        .border_type(BorderType::Rounded)
        .title(format!(" MEMÓRIAS & GUARDRAILS ({}) ", total));

    let table = Table::new(
        rows,
        [Constraint::Length(8), Constraint::Min(24), Constraint::Length(9)],
    )
    .header(header)
    .block(list_block);

    f.render_widget(table, split_chunks[0]);

    // Right Inspector
    let detail_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Theme::border_style())
        .border_type(BorderType::Rounded)
        .title(" DETALHES ");

    if let Some(item) = app.selected_item() {
        match item {
            DashboardItem::Memory(m) => {
                let badge_style = if m.is_invariant {
                    Style::default().fg(Theme::SUCCESS).add_modifier(Modifier::BOLD)
                } else if m.is_expired {
                    Style::default().fg(Theme::TEXT_MUTED)
                } else {
                    Style::default().fg(Theme::SECONDARY)
                };

                let badge_text = if m.is_invariant {
                    "Core (Permanente)"
                } else if m.is_expired {
                    "Expirado"
                } else {
                    "Ativo"
                };

                let details = vec![
                    Line::from(vec![
                        Span::styled("Item:   ", Style::default().fg(Theme::TEXT_MUTED)),
                        Span::styled(&m.title, Style::default().fg(Theme::TEXT).add_modifier(Modifier::BOLD)),
                    ]),
                    Line::from(vec![
                        Span::styled("Escopo: ", Style::default().fg(Theme::TEXT_MUTED)),
                        Span::styled(&m.scope, Style::default().fg(Theme::SECONDARY)),
                        Span::styled(" │ Status: ", Style::default().fg(Theme::TEXT_MUTED)),
                        Span::styled(badge_text, badge_style),
                    ]),
                    Line::from(""),
                    Line::from(Span::styled("── CONTEÚDO ──", Style::default().fg(Theme::TEXT_MUTED))),
                    Line::from(Span::styled(&m.title, Style::default().fg(Theme::TEXT))),
                    Line::from(""),
                    Line::from(vec![
                        Span::styled("Retenção: ", Style::default().fg(Theme::TEXT_MUTED)),
                        Span::styled(
                            if m.is_invariant { "100% (Congelada)".to_string() } else { format!("{:.1}%", m.ebbinghaus_retention * 100.0) },
                            Style::default().fg(if m.is_invariant { Theme::SUCCESS } else { Theme::TEXT })
                        ),
                    ]),
                    Line::from(vec![
                        Span::styled("Aprovação: ", Style::default().fg(Theme::TEXT_MUTED)),
                        Span::styled(if m.is_invariant { "Aprovado (HITL)" } else { "Automático" }, Style::default().fg(Theme::ACCENT)),
                    ]),
                    Line::from(""),
                    Line::from(Span::styled(app.db_path.to_string_lossy(), Style::default().fg(Theme::TEXT_MUTED))),
                ];

                let p = Paragraph::new(details).block(detail_block).wrap(Wrap { trim: true });
                f.render_widget(p, split_chunks[1]);
            }
            DashboardItem::AntiPattern(ap) => {
                let details = vec![
                    Line::from(vec![
                        Span::styled("Guardrail: ", Style::default().fg(Theme::TEXT_MUTED)),
                        Span::styled(&ap.pattern_name, Style::default().fg(Theme::TEXT).add_modifier(Modifier::BOLD)),
                    ]),
                    Line::from(vec![
                        Span::styled("Severidade: ", Style::default().fg(Theme::TEXT_MUTED)),
                        Span::styled(&ap.severity, Style::default().fg(Theme::DANGER).add_modifier(Modifier::BOLD)),
                        Span::styled(format!(" ({} ocorrências)", ap.occurrences), Style::default().fg(Theme::TEXT_MUTED)),
                    ]),
                    Line::from(""),
                    Line::from(Span::styled("── REGRA DE MITIGAÇÃO ──", Style::default().fg(Theme::TEXT_MUTED))),
                    Line::from(Span::styled(&ap.mitigation, Style::default().fg(Theme::SUCCESS).add_modifier(Modifier::BOLD))),
                    Line::from(""),
                    Line::from(Span::styled("── ASSINATURA DE ERRO ──", Style::default().fg(Theme::TEXT_MUTED))),
                    Line::from(Span::styled(&ap.signature, Style::default().fg(Theme::DANGER))),
                    Line::from(""),
                    Line::from(Span::styled(app.db_path.to_string_lossy(), Style::default().fg(Theme::TEXT_MUTED))),
                ];

                let p = Paragraph::new(details).block(detail_block).wrap(Wrap { trim: true });
                f.render_widget(p, split_chunks[1]);
            }
        }
    } else {
        let p = Paragraph::new("Nenhum registro carregado.").block(detail_block);
        f.render_widget(p, split_chunks[1]);
    }
}

fn render_footer(f: &mut Frame, _app: &App, area: Rect) {
    let shortcuts = Line::from(vec![
        Span::styled("↑/↓", Style::default().fg(Theme::PRIMARY).add_modifier(Modifier::BOLD)),
        Span::styled(" Navegar   ", Style::default().fg(Theme::TEXT_MUTED)),
        Span::styled("r", Style::default().fg(Theme::SUCCESS).add_modifier(Modifier::BOLD)),
        Span::styled(" Recarregar   ", Style::default().fg(Theme::TEXT_MUTED)),
        Span::styled("q", Style::default().fg(Theme::DANGER).add_modifier(Modifier::BOLD)),
        Span::styled(" Sair", Style::default().fg(Theme::TEXT_MUTED)),
    ]);

    let p = Paragraph::new(shortcuts).alignment(Alignment::Right);
    f.render_widget(p, area);
}
