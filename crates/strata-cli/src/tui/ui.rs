use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph, Tabs},
    Frame,
};

use crate::tui::app::{App, AppTab};
use crate::tui::theme::Theme;
use crate::tui::views::{anchors, antipatterns, jtms, memories, overview};

pub fn render(f: &mut Frame, app: &App) {
    let size = f.area();

    // Top-level vertical layout: Header (3), Tabs (3), Main Body (fill), Footer (2)
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Header
            Constraint::Length(3), // Tabs
            Constraint::Min(10),   // Content
            Constraint::Length(2), // Footer
        ])
        .split(size);

    render_header(f, app, chunks[0]);
    render_tabs(f, app, chunks[1]);
    render_content(f, app, chunks[2]);
    render_footer(f, app, chunks[3]);
}

fn render_header(f: &mut Frame, app: &App, area: Rect) {
    let header_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);

    // Left title: Strata Logo & Workspace
    let title_line = Line::from(vec![
        Span::styled("◆ STRATA COGNITIVE RUNTIME ", Style::default().fg(Theme::PRIMARY).add_modifier(Modifier::BOLD)),
        Span::styled("v0.1.0 ", Style::default().fg(Theme::TEXT_MUTED)),
        Span::styled("│ Workspace: ", Style::default().fg(Theme::BORDER_FOCUS)),
        Span::styled(&app.workspace_name, Style::default().fg(Theme::TEXT).add_modifier(Modifier::BOLD)),
    ]);

    let title_p = Paragraph::new(title_line)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Theme::border_style())
                .border_type(BorderType::Rounded),
        );
    f.render_widget(title_p, header_chunks[0]);

    // Right title: Persistence & Model State
    let status_line = Line::from(vec![
        Span::styled("[● SQLITE WAL ACTIVE] ", Style::default().fg(Theme::SUCCESS).add_modifier(Modifier::BOLD)),
        Span::styled("│ FastEmbed ONNX ", Style::default().fg(Theme::TEXT_MUTED)),
        Span::styled("│ JTMS v2", Style::default().fg(Theme::SECONDARY)),
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

fn render_tabs(f: &mut Frame, app: &App, area: Rect) {
    let titles: Vec<Line> = AppTab::all()
        .iter()
        .enumerate()
        .map(|(idx, tab)| {
            let is_selected = *tab == app.active_tab;
            let num = idx + 1;
            if is_selected {
                Line::from(vec![
                    Span::styled(format!(" [{num}] ", num = num), Style::default().fg(Theme::PRIMARY).add_modifier(Modifier::BOLD)),
                    Span::styled(tab.title(), Style::default().fg(Theme::TEXT).add_modifier(Modifier::BOLD)),
                    Span::raw(" "),
                ])
            } else {
                Line::from(vec![
                    Span::styled(format!(" [{num}] ", num = num), Style::default().fg(Theme::TEXT_MUTED)),
                    Span::styled(tab.title(), Style::default().fg(Theme::TEXT_MUTED)),
                    Span::raw(" "),
                ])
            }
        })
        .collect();

    let tabs = Tabs::new(titles)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Theme::focused_border_style())
                .border_type(BorderType::Rounded)
                .title(" NAVIGATION [Tab / 1-5] "),
        )
        .select(app.active_tab.to_index())
        .highlight_style(Theme::active_tab_style());

    f.render_widget(tabs, area);
}

fn render_content(f: &mut Frame, app: &App, area: Rect) {
    match app.active_tab {
        AppTab::Overview => overview::render(f, app, area),
        AppTab::Memories => memories::render(f, app, area),
        AppTab::AntiPatterns => antipatterns::render(f, app, area),
        AppTab::Jtms => jtms::render(f, app, area),
        AppTab::Anchors => anchors::render(f, app, area),
    }
}

fn render_footer(f: &mut Frame, app: &App, area: Rect) {
    let footer_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
        .split(area);

    let status_text = if let Some(ref msg) = app.status_message {
        msg.clone()
    } else {
        format!("Telemetry updated {} items loaded", app.report.total_memories)
    };

    let status_p = Paragraph::new(Line::from(vec![
        Span::styled("⚡ ", Style::default().fg(Theme::WARNING)),
        Span::styled(status_text, Style::default().fg(Theme::TEXT_MUTED)),
    ]));
    f.render_widget(status_p, footer_chunks[0]);

    let shortcuts_line = Line::from(vec![
        Span::styled("[Tab / Shift-Tab] ", Style::default().fg(Theme::PRIMARY).add_modifier(Modifier::BOLD)),
        Span::styled("Tab  ", Style::default().fg(Theme::TEXT)),
        Span::styled("[1-5] ", Style::default().fg(Theme::PRIMARY).add_modifier(Modifier::BOLD)),
        Span::styled("Direct  ", Style::default().fg(Theme::TEXT)),
        Span::styled("[↑/↓ or j/k] ", Style::default().fg(Theme::SECONDARY).add_modifier(Modifier::BOLD)),
        Span::styled("Select  ", Style::default().fg(Theme::TEXT)),
        Span::styled("[r] ", Style::default().fg(Theme::SUCCESS).add_modifier(Modifier::BOLD)),
        Span::styled("Refresh  ", Style::default().fg(Theme::TEXT)),
        Span::styled("[q / Esc] ", Style::default().fg(Theme::DANGER).add_modifier(Modifier::BOLD)),
        Span::styled("Quit", Style::default().fg(Theme::TEXT)),
    ]);

    let shortcuts_p = Paragraph::new(shortcuts_line).alignment(Alignment::Right);
    f.render_widget(shortcuts_p, footer_chunks[1]);
}
