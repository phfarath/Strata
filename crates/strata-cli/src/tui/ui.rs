use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Cell, Gauge, Paragraph, Row, Table, Tabs, Wrap},
    Frame,
};

use crate::tui::app::{App, AppTab, DashboardItem};
use crate::tui::theme::Theme;
use chrono::Utc;
use strata_core::blackboard::TaskStatus;

pub fn render(f: &mut Frame, app: &App) {
    let size = f.area();

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Header
            Constraint::Length(3), // Navigation Tabs
            Constraint::Length(4), // KPIs + Retention / Progress Bar
            Constraint::Min(8),    // 50/50 Split: Items | Inspector OR Swarm Split
            Constraint::Length(1), // Minimal Footer
        ])
        .split(size);

    render_header(f, app, chunks[0]);
    render_tabs(f, app, chunks[1]);

    match app.active_tab {
        AppTab::Swarm => {
            render_swarm_kpis(f, app, chunks[2]);
            render_swarm_split(f, app, chunks[3]);
        }
        _ => {
            render_kpis(f, app, chunks[2]);
            render_split(f, app, chunks[3]);
        }
    }

    render_footer(f, app, chunks[4]);
}

fn render_tabs(f: &mut Frame, app: &App, area: Rect) {
    let tab_titles: Vec<Line> = AppTab::all()
        .iter()
        .enumerate()
        .map(|(i, tab)| Line::from(format!(" {}: {} ", i + 1, tab.title())))
        .collect();

    let tabs = Tabs::new(tab_titles)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Theme::border_style())
                .border_type(BorderType::Rounded)
                .title(" NAVEGAÇÃO [1-6 / ←→ / h,l] "),
        )
        .select(app.active_tab.to_index())
        .style(Theme::inactive_tab_style())
        .highlight_style(Theme::active_tab_style())
        .divider("│");

    f.render_widget(tabs, area);
}

fn render_header(f: &mut Frame, app: &App, area: Rect) {
    let header_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(70), Constraint::Percentage(30)])
        .split(area);

    let title_line = Line::from(vec![
        Span::styled(
            "STRATA",
            Style::default()
                .fg(Theme::PRIMARY)
                .add_modifier(Modifier::BOLD),
        ),
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
        Span::styled(
            "Local",
            Style::default()
                .fg(Theme::SUCCESS)
                .add_modifier(Modifier::BOLD),
        ),
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
    let invariants = app
        .report
        .memories
        .iter()
        .filter(|m| m.is_invariant)
        .count();
    let guardrails = app.report.anti_patterns.len();

    let active_mems = app.report.memories.iter().filter(|m| !m.is_expired).count();
    let avg_retention = if total_mems > 0 {
        (active_mems as f32 / total_mems as f32) * 100.0
    } else {
        100.0
    };

    let p1 = Paragraph::new(Line::from(vec![
        Span::styled("Memórias: ", Style::default().fg(Theme::TEXT_MUTED)),
        Span::styled(
            format!("{}", total_mems),
            Style::default()
                .fg(Theme::TEXT)
                .add_modifier(Modifier::BOLD),
        ),
    ]));
    let p2 = Paragraph::new(Line::from(vec![
        Span::styled("Core: ", Style::default().fg(Theme::TEXT_MUTED)),
        Span::styled(
            format!("{}", invariants),
            Style::default()
                .fg(Theme::SUCCESS)
                .add_modifier(Modifier::BOLD),
        ),
    ]));
    let p3 = Paragraph::new(Line::from(vec![
        Span::styled("Guardrails: ", Style::default().fg(Theme::TEXT_MUTED)),
        Span::styled(
            format!("{}", guardrails),
            Style::default()
                .fg(Theme::DANGER)
                .add_modifier(Modifier::BOLD),
        ),
    ]));
    let p4 = Paragraph::new(Line::from(vec![
        Span::styled("Retenção: ", Style::default().fg(Theme::TEXT_MUTED)),
        Span::styled(
            format!("{:.0}%", avg_retention),
            Style::default()
                .fg(Theme::PRIMARY)
                .add_modifier(Modifier::BOLD),
        ),
    ]));

    f.render_widget(p1, cards[0]);
    f.render_widget(p2, cards[1]);
    f.render_widget(p3, cards[2]);
    f.render_widget(p4, cards[3]);

    let gauge_ratio = (avg_retention / 100.0).clamp(0.0, 1.0);
    let gauge = Gauge::default()
        .gauge_style(
            Style::default()
                .fg(Theme::SUCCESS)
                .bg(Color::Rgb(30, 41, 59)),
        )
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
    let header_cells = ["Status", "Item", "Retenção"].iter().map(|h| {
        Cell::from(*h).style(
            Style::default()
                .fg(Theme::PRIMARY)
                .add_modifier(Modifier::BOLD),
        )
    });
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
                    (
                        "[CORE]",
                        Style::default()
                            .fg(Theme::SUCCESS)
                            .add_modifier(Modifier::BOLD),
                    )
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
                let status_style = Style::default()
                    .fg(Theme::DANGER)
                    .add_modifier(Modifier::BOLD);
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
        [
            Constraint::Length(8),
            Constraint::Min(24),
            Constraint::Length(9),
        ],
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
                    Style::default()
                        .fg(Theme::SUCCESS)
                        .add_modifier(Modifier::BOLD)
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
                        Span::styled(
                            &m.title,
                            Style::default()
                                .fg(Theme::TEXT)
                                .add_modifier(Modifier::BOLD),
                        ),
                    ]),
                    Line::from(vec![
                        Span::styled("Escopo: ", Style::default().fg(Theme::TEXT_MUTED)),
                        Span::styled(&m.scope, Style::default().fg(Theme::SECONDARY)),
                        Span::styled(" │ Status: ", Style::default().fg(Theme::TEXT_MUTED)),
                        Span::styled(badge_text, badge_style),
                    ]),
                    Line::from(""),
                    Line::from(Span::styled(
                        "── CONTEÚDO ──",
                        Style::default().fg(Theme::TEXT_MUTED),
                    )),
                    Line::from(Span::styled(&m.title, Style::default().fg(Theme::TEXT))),
                    Line::from(""),
                    Line::from(vec![
                        Span::styled("Retenção: ", Style::default().fg(Theme::TEXT_MUTED)),
                        Span::styled(
                            if m.is_invariant {
                                "100% (Congelada)".to_string()
                            } else {
                                format!("{:.1}%", m.ebbinghaus_retention * 100.0)
                            },
                            Style::default().fg(if m.is_invariant {
                                Theme::SUCCESS
                            } else {
                                Theme::TEXT
                            }),
                        ),
                    ]),
                    Line::from(vec![
                        Span::styled("Aprovação: ", Style::default().fg(Theme::TEXT_MUTED)),
                        Span::styled(
                            if m.is_invariant {
                                "Aprovado (HITL)"
                            } else {
                                "Automático"
                            },
                            Style::default().fg(Theme::ACCENT),
                        ),
                    ]),
                    Line::from(""),
                    Line::from(Span::styled(
                        app.db_path.to_string_lossy(),
                        Style::default().fg(Theme::TEXT_MUTED),
                    )),
                ];

                let p = Paragraph::new(details)
                    .block(detail_block)
                    .wrap(Wrap { trim: true });
                f.render_widget(p, split_chunks[1]);
            }
            DashboardItem::AntiPattern(ap) => {
                let details = vec![
                    Line::from(vec![
                        Span::styled("Guardrail: ", Style::default().fg(Theme::TEXT_MUTED)),
                        Span::styled(
                            &ap.pattern_name,
                            Style::default()
                                .fg(Theme::TEXT)
                                .add_modifier(Modifier::BOLD),
                        ),
                    ]),
                    Line::from(vec![
                        Span::styled("Severidade: ", Style::default().fg(Theme::TEXT_MUTED)),
                        Span::styled(
                            &ap.severity,
                            Style::default()
                                .fg(Theme::DANGER)
                                .add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(
                            format!(" ({} ocorrências)", ap.occurrences),
                            Style::default().fg(Theme::TEXT_MUTED),
                        ),
                    ]),
                    Line::from(""),
                    Line::from(Span::styled(
                        "── REGRA DE MITIGAÇÃO ──",
                        Style::default().fg(Theme::TEXT_MUTED),
                    )),
                    Line::from(Span::styled(
                        &ap.mitigation,
                        Style::default()
                            .fg(Theme::SUCCESS)
                            .add_modifier(Modifier::BOLD),
                    )),
                    Line::from(""),
                    Line::from(Span::styled(
                        "── ASSINATURA DE ERRO ──",
                        Style::default().fg(Theme::TEXT_MUTED),
                    )),
                    Line::from(Span::styled(
                        &ap.signature,
                        Style::default().fg(Theme::DANGER),
                    )),
                    Line::from(""),
                    Line::from(Span::styled(
                        app.db_path.to_string_lossy(),
                        Style::default().fg(Theme::TEXT_MUTED),
                    )),
                ];

                let p = Paragraph::new(details)
                    .block(detail_block)
                    .wrap(Wrap { trim: true });
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
        Span::styled(
            "←/→/h/l",
            Style::default()
                .fg(Theme::PRIMARY)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" Abas   ", Style::default().fg(Theme::TEXT_MUTED)),
        Span::styled(
            "1-6",
            Style::default()
                .fg(Theme::PRIMARY)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" Atalho   ", Style::default().fg(Theme::TEXT_MUTED)),
        Span::styled(
            "↑/↓/j/k/Tab",
            Style::default()
                .fg(Theme::PRIMARY)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" Navegar   ", Style::default().fg(Theme::TEXT_MUTED)),
        Span::styled(
            "r",
            Style::default()
                .fg(Theme::SUCCESS)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" Recarregar   ", Style::default().fg(Theme::TEXT_MUTED)),
        Span::styled(
            "q",
            Style::default()
                .fg(Theme::DANGER)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" Sair", Style::default().fg(Theme::TEXT_MUTED)),
    ]);

    let p = Paragraph::new(shortcuts).alignment(Alignment::Right);
    f.render_widget(p, area);
}

fn render_swarm_kpis(f: &mut Frame, app: &App, area: Rect) {
    let kpi_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(2), Constraint::Length(2)])
        .split(area);

    let cards = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(20),
            Constraint::Percentage(20),
            Constraint::Percentage(20),
            Constraint::Percentage(20),
            Constraint::Percentage(20),
        ])
        .split(kpi_chunks[0]);

    let total_tasks = app.tasks.len();
    let pending_tasks = app
        .tasks
        .iter()
        .filter(|t| t.status == TaskStatus::Pending || t.status == TaskStatus::Blocked)
        .count();
    let running_tasks = app
        .tasks
        .iter()
        .filter(|t| t.status == TaskStatus::Running || t.status == TaskStatus::Claimed)
        .count();
    let completed_tasks = app
        .tasks
        .iter()
        .filter(|t| t.status == TaskStatus::Completed)
        .count();
    let now = Utc::now().timestamp();
    let active_leases = app.leases.iter().filter(|l| !l.is_expired(now)).count();

    let p1 = Paragraph::new(Line::from(vec![
        Span::styled("Total Tasks: ", Style::default().fg(Theme::TEXT_MUTED)),
        Span::styled(
            format!("{}", total_tasks),
            Style::default()
                .fg(Theme::TEXT)
                .add_modifier(Modifier::BOLD),
        ),
    ]));
    let p2 = Paragraph::new(Line::from(vec![
        Span::styled("Pendentes: ", Style::default().fg(Theme::TEXT_MUTED)),
        Span::styled(
            format!("{}", pending_tasks),
            Style::default()
                .fg(Theme::WARNING)
                .add_modifier(Modifier::BOLD),
        ),
    ]));
    let p3 = Paragraph::new(Line::from(vec![
        Span::styled("Em Execução: ", Style::default().fg(Theme::TEXT_MUTED)),
        Span::styled(
            format!("{}", running_tasks),
            Style::default()
                .fg(Theme::SECONDARY)
                .add_modifier(Modifier::BOLD),
        ),
    ]));
    let p4 = Paragraph::new(Line::from(vec![
        Span::styled("Concluídas: ", Style::default().fg(Theme::TEXT_MUTED)),
        Span::styled(
            format!("{}", completed_tasks),
            Style::default()
                .fg(Theme::SUCCESS)
                .add_modifier(Modifier::BOLD),
        ),
    ]));
    let p5 = Paragraph::new(Line::from(vec![
        Span::styled("Leases Ativas: ", Style::default().fg(Theme::TEXT_MUTED)),
        Span::styled(
            format!("{}", active_leases),
            Style::default()
                .fg(Theme::ACCENT)
                .add_modifier(Modifier::BOLD),
        ),
    ]));

    f.render_widget(p1, cards[0]);
    f.render_widget(p2, cards[1]);
    f.render_widget(p3, cards[2]);
    f.render_widget(p4, cards[3]);
    f.render_widget(p5, cards[4]);

    let progress_ratio = if total_tasks > 0 {
        (completed_tasks as f64 / total_tasks as f64).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let gauge = Gauge::default()
        .gauge_style(
            Style::default()
                .fg(Theme::PRIMARY)
                .bg(Color::Rgb(30, 41, 59)),
        )
        .ratio(progress_ratio)
        .label(format!(
            "Progresso do Swarm: {}/{} Concluídas ({:.1}%)",
            completed_tasks,
            total_tasks,
            progress_ratio * 100.0
        ));
    f.render_widget(gauge, kpi_chunks[1]);
}

fn render_swarm_split(f: &mut Frame, app: &App, area: Rect) {
    let split_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
        .split(area);

    render_swarm_tasks(f, app, split_chunks[0]);

    let right_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
        .split(split_chunks[1]);

    render_swarm_leases(f, app, right_chunks[0]);
    render_swarm_task_details(f, app, right_chunks[1]);
}

fn render_swarm_tasks(f: &mut Frame, app: &App, area: Rect) {
    let header_cells = ["Status", "ID", "Título", "Agente", "Deps"].iter().map(|h| {
        Cell::from(*h).style(
            Style::default()
                .fg(Theme::PRIMARY)
                .add_modifier(Modifier::BOLD),
        )
    });
    let header = Row::new(header_cells).height(1).bottom_margin(1);

    let rows = app.tasks.iter().enumerate().map(|(idx, t)| {
        let is_selected = idx == app.selected_task_idx;
        let row_style = if is_selected {
            Theme::selected_row_style()
        } else {
            Style::default().fg(Theme::TEXT)
        };

        let (status_tag, status_style) = match t.status {
            TaskStatus::Pending => ("[PEND]", Style::default().fg(Theme::WARNING)),
            TaskStatus::Claimed => ("[CLMD]", Style::default().fg(Theme::SECONDARY)),
            TaskStatus::Running => (
                "[RUN ]",
                Style::default()
                    .fg(Theme::SECONDARY)
                    .add_modifier(Modifier::BOLD),
            ),
            TaskStatus::Blocked => ("[BLCK]", Style::default().fg(Theme::TEXT_MUTED)),
            TaskStatus::Completed => (
                "[DONE]",
                Style::default()
                    .fg(Theme::SUCCESS)
                    .add_modifier(Modifier::BOLD),
            ),
            TaskStatus::Failed => (
                "[FAIL]",
                Style::default()
                    .fg(Theme::DANGER)
                    .add_modifier(Modifier::BOLD),
            ),
        };

        let title_short: String = t.title.chars().take(28).collect();
        let agent_str = t.assigned_agent.as_deref().unwrap_or("-");
        let deps_str = if t.dependencies.is_empty() {
            "-".to_string()
        } else {
            t.dependencies.join(",")
        };

        Row::new(vec![
            Cell::from(status_tag).style(status_style),
            Cell::from(t.task_id.clone()),
            Cell::from(title_short),
            Cell::from(agent_str.to_string()),
            Cell::from(deps_str),
        ])
        .style(row_style)
        .height(1)
    });

    let list_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Theme::focused_border_style())
        .border_type(BorderType::Rounded)
        .title(format!(" BLACKBOARD TASKS ({}) ", app.tasks.len()));

    let table = Table::new(
        rows,
        [
            Constraint::Length(8),
            Constraint::Length(14),
            Constraint::Min(16),
            Constraint::Length(14),
            Constraint::Length(10),
        ],
    )
    .header(header)
    .block(list_block);

    f.render_widget(table, area);
}

fn render_swarm_leases(f: &mut Frame, app: &App, area: Rect) {
    let now = Utc::now().timestamp();
    let header_cells = ["Recurso", "Agente", "TTL"].iter().map(|h| {
        Cell::from(*h).style(
            Style::default()
                .fg(Theme::PRIMARY)
                .add_modifier(Modifier::BOLD),
        )
    });
    let header = Row::new(header_cells).height(1).bottom_margin(1);

    let rows = app.leases.iter().map(|lease| {
        let remaining = lease.remaining_seconds(now);
        let ttl_style = if remaining > 60 {
            Style::default().fg(Theme::SUCCESS)
        } else if remaining > 10 {
            Style::default().fg(Theme::WARNING)
        } else {
            Style::default().fg(Theme::DANGER)
        };

        Row::new(vec![
            Cell::from(lease.resource_id.clone()),
            Cell::from(lease.agent_id.clone()),
            Cell::from(format!("{}s", remaining)).style(ttl_style),
        ])
        .height(1)
    });

    let leases_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Theme::border_style())
        .border_type(BorderType::Rounded)
        .title(format!(" LEASES DE RECURSOS ATIVAS ({}) ", app.leases.len()));

    let table = Table::new(
        rows,
        [
            Constraint::Percentage(50),
            Constraint::Percentage(30),
            Constraint::Percentage(20),
        ],
    )
    .header(header)
    .block(leases_block);

    f.render_widget(table, area);
}

fn render_swarm_task_details(f: &mut Frame, app: &App, area: Rect) {
    let detail_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Theme::border_style())
        .border_type(BorderType::Rounded)
        .title(" DETALHES DA TASK ");

    if let Some(task) = app.selected_task() {
        let (status_badge, status_style) = match task.status {
            TaskStatus::Pending => ("[PENDENTE]", Style::default().fg(Theme::WARNING)),
            TaskStatus::Claimed => ("[RESERVADA]", Style::default().fg(Theme::SECONDARY)),
            TaskStatus::Running => (
                "[EXECUTANDO]",
                Style::default()
                    .fg(Theme::SECONDARY)
                    .add_modifier(Modifier::BOLD),
            ),
            TaskStatus::Blocked => ("[BLOQUEADA]", Style::default().fg(Theme::TEXT_MUTED)),
            TaskStatus::Completed => (
                "[CONCLUÍDA]",
                Style::default()
                    .fg(Theme::SUCCESS)
                    .add_modifier(Modifier::BOLD),
            ),
            TaskStatus::Failed => (
                "[FALHA]",
                Style::default()
                    .fg(Theme::DANGER)
                    .add_modifier(Modifier::BOLD),
            ),
        };

        let mut lines = vec![
            Line::from(vec![
                Span::styled("ID:          ", Style::default().fg(Theme::TEXT_MUTED)),
                Span::styled(
                    &task.task_id,
                    Style::default()
                        .fg(Theme::PRIMARY)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(" │ Status: ", Style::default().fg(Theme::TEXT_MUTED)),
                Span::styled(status_badge, status_style),
            ]),
            Line::from(vec![
                Span::styled("Título:      ", Style::default().fg(Theme::TEXT_MUTED)),
                Span::styled(
                    &task.title,
                    Style::default()
                        .fg(Theme::TEXT)
                        .add_modifier(Modifier::BOLD),
                ),
            ]),
        ];

        if let Some(desc) = &task.description {
            lines.push(Line::from(vec![
                Span::styled("Descrição:   ", Style::default().fg(Theme::TEXT_MUTED)),
                Span::styled(desc, Style::default().fg(Theme::TEXT)),
            ]));
        }

        let agent = task.assigned_agent.as_deref().unwrap_or("Não atribuído");
        lines.push(Line::from(vec![
            Span::styled("Agente:      ", Style::default().fg(Theme::TEXT_MUTED)),
            Span::styled(agent, Style::default().fg(Theme::SECONDARY)),
        ]));

        if let Some(plan_id) = &task.plan_id {
            lines.push(Line::from(vec![
                Span::styled("Plano ID:    ", Style::default().fg(Theme::TEXT_MUTED)),
                Span::styled(plan_id, Style::default().fg(Theme::TEXT_MUTED)),
            ]));
        }

        let req_resources = if task.required_resources.is_empty() {
            "Nenhum".to_string()
        } else {
            task.required_resources.join(", ")
        };
        lines.push(Line::from(vec![
            Span::styled("Recursos:    ", Style::default().fg(Theme::TEXT_MUTED)),
            Span::styled(req_resources, Style::default().fg(Theme::ACCENT)),
        ]));

        let deps = if task.dependencies.is_empty() {
            "Nenhuma".to_string()
        } else {
            task.dependencies.join(", ")
        };
        lines.push(Line::from(vec![
            Span::styled("Depende de:  ", Style::default().fg(Theme::TEXT_MUTED)),
            Span::styled(deps, Style::default().fg(Theme::WARNING)),
        ]));

        if let Some(outputs) = &task.outputs {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "── OUTPUTS / ARTEFATOS ──",
                Style::default().fg(Theme::TEXT_MUTED),
            )));
            let json_str =
                serde_json::to_string_pretty(outputs).unwrap_or_else(|_| outputs.to_string());
            for l in json_str.lines().take(8) {
                lines.push(Line::from(Span::styled(
                    l.to_string(),
                    Style::default().fg(Theme::SUCCESS),
                )));
            }
        }

        let p = Paragraph::new(lines)
            .block(detail_block)
            .wrap(Wrap { trim: true });
        f.render_widget(p, area);
    } else {
        let p = Paragraph::new("Nenhuma task selecionada no Blackboard.")
            .block(detail_block)
            .wrap(Wrap { trim: true });
        f.render_widget(p, area);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    use std::path::PathBuf;
    use std::sync::Arc;
    use strata_core::a2a::ResourceLease;
    use strata_core::blackboard::{TaskRecord, TaskStatus};
    use strata_memory::SqliteMemoryEngine;

    #[test]
    fn test_render_overview_and_swarm_tabs() {
        let engine = Arc::new(SqliteMemoryEngine::open_in_memory(None).expect("engine"));
        let mut app = App::new(engine, PathBuf::from("test.db"), "TestWorkspace".to_string())
            .expect("app new");

        // Add dummy data for Swarm
        let mut task = TaskRecord::new("task-42", "Refactor core memory");
        task.status = TaskStatus::Running;
        task.assigned_agent = Some("agent-007".to_string());
        task.required_resources = vec!["core:memory".to_string()];
        task.dependencies = vec!["task-41".to_string()];
        task.outputs = Some(serde_json::json!({"status": "in_progress"}));
        app.tasks.push(task);

        let lease = ResourceLease::new("core:memory", "agent-007", Utc::now().timestamp() + 300);
        app.leases.push(lease);

        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend).expect("terminal");

        // Test rendering Overview
        app.set_tab(AppTab::Overview);
        terminal.draw(|f| render(f, &app)).expect("draw overview");

        // Test rendering Swarm Cockpit
        app.set_tab(AppTab::Swarm);
        terminal.draw(|f| render(f, &app)).expect("draw swarm");
    }
}

