use std::path::PathBuf;
use std::sync::Arc;
use anyhow::Result;
use strata_memory::{SqliteMemoryEngine, SqliteStore};
use crate::commands::observe::{generate_report, AntiPatternItem, CognitiveReport, MemoryDecayItem, ObserveArgs};

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
        &[AppTab::Overview, AppTab::Memories, AppTab::AntiPatterns, AppTab::Jtms, AppTab::Anchors]
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

pub enum DashboardItem<'a> {
    Memory(&'a MemoryDecayItem),
    AntiPattern(&'a AntiPatternItem),
}

pub struct App {
    pub active_tab: AppTab,
    pub report: CognitiveReport,
    pub workspace_name: String,
    pub db_path: PathBuf,
    pub selected_idx: usize,
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
            selected_idx: 0,
            should_quit: false,
            status_message: None,
        })
    }

    pub fn total_items(&self) -> usize {
        self.report.memories.len() + self.report.anti_patterns.len()
    }

    pub fn get_item(&self, index: usize) -> Option<DashboardItem<'_>> {
        let mem_count = self.report.memories.len();
        if index < mem_count {
            self.report.memories.get(index).map(DashboardItem::Memory)
        } else {
            let ap_index = index - mem_count;
            self.report.anti_patterns.get(ap_index).map(DashboardItem::AntiPattern)
        }
    }

    pub fn selected_item(&self) -> Option<DashboardItem<'_>> {
        self.get_item(self.selected_idx)
    }

    pub fn next_row(&mut self) {
        let total = self.total_items();
        if total > 0 {
            self.selected_idx = (self.selected_idx + 1) % total;
        }
    }

    pub fn prev_row(&mut self) {
        let total = self.total_items();
        if total > 0 {
            if self.selected_idx == 0 {
                self.selected_idx = total - 1;
            } else {
                self.selected_idx -= 1;
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
        if self.selected_idx >= self.total_items() && self.total_items() > 0 {
            self.selected_idx = self.total_items() - 1;
        }
        self.status_message = Some("Recarregado.".to_string());
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
