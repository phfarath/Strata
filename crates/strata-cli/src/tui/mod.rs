pub mod app;
pub mod theme;
pub mod ui;

use std::io;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use crossterm::{
    cursor::{Hide, Show},
    event::{self, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};
use strata_memory::SqliteMemoryEngine;

use self::app::App;

/// RAII Guard ensuring terminal raw mode and alternate screen are always cleaned up
struct TerminalGuard;

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen, Show);
    }
}

/// Run the interactive Terminal UI (TUI) dashboard.
pub async fn run_tui(engine: Arc<SqliteMemoryEngine>, db_path: &Path) -> Result<()> {
    // 1. Setup panic hook to restore terminal on crash
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen, Show);
        original_hook(panic_info);
    }));

    // 2. Enable raw mode and alternate screen
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, Hide)?;
    let _guard = TerminalGuard;

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;

    // 3. Resolve workspace name
    let workspace_name = std::env::current_dir()
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
        .unwrap_or_else(|| "Strata Workspace".to_string());

    // 4. Initialize application state
    let mut app = App::new(engine.clone(), db_path.to_path_buf(), workspace_name)?;

    // 5. Main event loop
    loop {
        terminal.draw(|f| ui::render(f, &app))?;

        // 200ms event tick rate
        if event::poll(Duration::from_millis(200))? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    match key.code {
                        KeyCode::Char('q') | KeyCode::Esc => {
                            app.quit();
                        }
                        KeyCode::Tab | KeyCode::Char('j') | KeyCode::Down => {
                            app.next_row();
                        }
                        KeyCode::BackTab | KeyCode::Char('k') | KeyCode::Up => {
                            app.prev_row();
                        }
                        KeyCode::Char('r') => {
                            let _ = app.refresh(engine.store());
                        }
                        _ => {}
                    }
                }
            }
        }

        if app.should_quit {
            break;
        }
    }

    Ok(())
}
