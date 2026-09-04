use std::path::PathBuf;
use std::sync::Arc;
use anyhow::Result;
use strata_memory::{SqliteMemoryEngine, SqliteStore};
use crate::commands::observe::{generate_report, CognitiveReport, ObserveArgs};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppTab {
    Overview = 0,
    Memories = 1,
    AntiPatterns = 2,
    Jtms = 3,
    Anchors = 4,
}

impl AppTab {
    pub fn all() -> &'static [AppTab] {
        &[
            AppTab::Overview,
            AppTab::Memories,
            AppTab::AntiPatterns,
            AppTab::Jtms,
            AppTab::Anchors,
        ]
    }

    pub fn title(&self) -> &'static str {
        match self {
            AppTab::Overview => "Overview",
            AppTab::Memories => "Memories & Decay",
            AppTab::AntiPatterns => "Anti-Patterns",
            AppTab::Jtms => "JTMS Truth Graph",
            AppTab::Anchors => "AST Code Anchors",
        }
    }

    pub fn from_index(index: usize) -> Self {
        match index {
            0 => AppTab::Overview,
            1 => AppTab::Memories,
            2 => AppTab::AntiPatterns,
            3 => AppTab::Jtms,
            4 => AppTab::Anchors,
            _ => AppTab::Overview,
        }
    }

    pub fn to_index(&self) -> usize {
        *self as usize
    }
}

pub struct App {
    pub active_tab: AppTab,
    pub report: CognitiveReport,
    pub workspace_name: String,
    pub db_path: PathBuf,
    pub selected_memory_idx: usize,
    pub selected_antipattern_idx: usize,
    pub selected_jtms_idx: usize,
    pub selected_anchor_idx: usize,
    pub should_quit: bool,
    pub status_message: Option<String>,
}

impl App {
    pub fn new(
        engine: Arc<SqliteMemoryEngine>,
        db_path: PathBuf,
        workspace_name: String,
    ) -> Result<Self> {
        let store = engine.store();
        let args = ObserveArgs {
            live: false,
            interval_secs: 2,
            tab: None,
            scope: None,
            horizon_hours: 168.0,
            limit: 50,
            json: false,
        };

        let report = generate_report(&store, &args)?;

        Ok(Self {
            active_tab: AppTab::Overview,
            report,
            workspace_name,
            db_path,
            selected_memory_idx: 0,
            selected_antipattern_idx: 0,
            selected_jtms_idx: 0,
            selected_anchor_idx: 0,
            should_quit: false,
            status_message: None,
        })
    }

    pub fn next_tab(&mut self) {
        let current = self.active_tab.to_index();
        let next = (current + 1) % AppTab::all().len();
        self.active_tab = AppTab::from_index(next);
    }

    pub fn prev_tab(&mut self) {
        let current = self.active_tab.to_index();
        let prev = if current == 0 {
            AppTab::all().len() - 1
        } else {
            current - 1
        };
        self.active_tab = AppTab::from_index(prev);
    }

    pub fn set_tab(&mut self, index: usize) {
        if index < AppTab::all().len() {
            self.active_tab = AppTab::from_index(index);
        }
    }

    pub fn next_row(&mut self) {
        match self.active_tab {
            AppTab::Overview => {}
            AppTab::Memories => {
                if !self.report.memories.is_empty() {
                    self.selected_memory_idx = (self.selected_memory_idx + 1) % self.report.memories.len();
                }
            }
            AppTab::AntiPatterns => {
                if !self.report.anti_patterns.is_empty() {
                    self.selected_antipattern_idx = (self.selected_antipattern_idx + 1) % self.report.anti_patterns.len();
                }
            }
            AppTab::Jtms => {
                if !self.report.memories.is_empty() {
                    self.selected_jtms_idx = (self.selected_jtms_idx + 1) % self.report.memories.len();
                }
            }
            AppTab::Anchors => {
                if !self.report.memories.is_empty() {
                    self.selected_anchor_idx = (self.selected_anchor_idx + 1) % self.report.memories.len();
                }
            }
        }
    }

    pub fn prev_row(&mut self) {
        match self.active_tab {
            AppTab::Overview => {}
            AppTab::Memories => {
                if !self.report.memories.is_empty() {
                    if self.selected_memory_idx == 0 {
                        self.selected_memory_idx = self.report.memories.len() - 1;
                    } else {
                        self.selected_memory_idx -= 1;
                    }
                }
            }
            AppTab::AntiPatterns => {
                if !self.report.anti_patterns.is_empty() {
                    if self.selected_antipattern_idx == 0 {
                        self.selected_antipattern_idx = self.report.anti_patterns.len() - 1;
                    } else {
                        self.selected_antipattern_idx -= 1;
                    }
                }
            }
            AppTab::Jtms => {
                if !self.report.memories.is_empty() {
                    if self.selected_jtms_idx == 0 {
                        self.selected_jtms_idx = self.report.memories.len() - 1;
                    } else {
                        self.selected_jtms_idx -= 1;
                    }
                }
            }
            AppTab::Anchors => {
                if !self.report.memories.is_empty() {
                    if self.selected_anchor_idx == 0 {
                        self.selected_anchor_idx = self.report.memories.len() - 1;
                    } else {
                        self.selected_anchor_idx -= 1;
                    }
                }
            }
        }
    }

    pub fn refresh(&mut self, store: &SqliteStore) -> Result<()> {
        let args = ObserveArgs {
            live: false,
            interval_secs: 2,
            tab: None,
            scope: None,
            horizon_hours: 168.0,
            limit: 50,
            json: false,
        };
        self.report = generate_report(store, &args)?;
        self.status_message = Some("Refreshed cognitive state.".to_string());
        Ok(())
    }

    pub fn quit(&mut self) {
        self.should_quit = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_app_tab_navigation() {
        assert_eq!(AppTab::from_index(0), AppTab::Overview);
        assert_eq!(AppTab::from_index(1), AppTab::Memories);
        assert_eq!(AppTab::from_index(2), AppTab::AntiPatterns);
        assert_eq!(AppTab::from_index(3), AppTab::Jtms);
        assert_eq!(AppTab::from_index(4), AppTab::Anchors);
        assert_eq!(AppTab::from_index(99), AppTab::Overview);

        assert_eq!(AppTab::Overview.title(), "Overview");
        assert_eq!(AppTab::Memories.title(), "Memories & Decay");
    }
}
