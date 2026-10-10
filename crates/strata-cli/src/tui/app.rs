use crate::commands::observe::{
    generate_report, AntiPatternItem, CognitiveReport, MemoryDecayItem, ObserveArgs,
};
use anyhow::Result;
use std::path::PathBuf;
use std::sync::Arc;
use strata_core::a2a::ResourceLease;
use strata_core::blackboard::{TaskFilter, TaskRecord};
use strata_memory::{SqliteMemoryEngine, SqliteStore};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppTab {
    Overview = 0,
    Swarm = 1,
    Memories = 2,
    AntiPatterns = 3,
    Jtms = 4,
    Anchors = 5,
}

impl AppTab {
    pub fn all() -> &'static [AppTab] {
        &[
            AppTab::Overview,
            AppTab::Swarm,
            AppTab::Memories,
            AppTab::AntiPatterns,
            AppTab::Jtms,
            AppTab::Anchors,
        ]
    }

    pub fn title(&self) -> &'static str {
        match self {
            AppTab::Overview => "Overview",
            AppTab::Swarm => "Swarm Cockpit",
            AppTab::Memories => "Memories & Decay",
            AppTab::AntiPatterns => "Anti-Patterns",
            AppTab::Jtms => "JTMS Truth Graph",
            AppTab::Anchors => "AST Code Anchors",
        }
    }

    pub fn from_index(index: usize) -> Self {
        match index {
            0 => AppTab::Overview,
            1 => AppTab::Swarm,
            2 => AppTab::Memories,
            3 => AppTab::AntiPatterns,
            4 => AppTab::Jtms,
            5 => AppTab::Anchors,
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
    pub tasks: Vec<TaskRecord>,
    pub leases: Vec<ResourceLease>,
    pub selected_task_idx: usize,
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

        let report = generate_report(store, &args)?;
        let tasks = store.list_tasks(&TaskFilter::default())?;
        let leases = store.list_active_leases()?;

        Ok(Self {
            active_tab: AppTab::Overview,
            report,
            workspace_name,
            db_path,
            selected_idx: 0,
            tasks,
            leases,
            selected_task_idx: 0,
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
            self.report
                .anti_patterns
                .get(ap_index)
                .map(DashboardItem::AntiPattern)
        }
    }

    pub fn selected_item(&self) -> Option<DashboardItem<'_>> {
        self.get_item(self.selected_idx)
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

    pub fn set_tab(&mut self, tab: AppTab) {
        self.active_tab = tab;
    }

    pub fn next_row(&mut self) {
        if self.active_tab == AppTab::Swarm {
            if !self.tasks.is_empty() {
                self.selected_task_idx = (self.selected_task_idx + 1) % self.tasks.len();
            }
        } else {
            let total = self.total_items();
            if total > 0 {
                self.selected_idx = (self.selected_idx + 1) % total;
            }
        }
    }

    pub fn prev_row(&mut self) {
        if self.active_tab == AppTab::Swarm {
            if !self.tasks.is_empty() {
                if self.selected_task_idx == 0 {
                    self.selected_task_idx = self.tasks.len() - 1;
                } else {
                    self.selected_task_idx -= 1;
                }
            }
        } else {
            let total = self.total_items();
            if total > 0 {
                if self.selected_idx == 0 {
                    self.selected_idx = total - 1;
                } else {
                    self.selected_idx -= 1;
                }
            }
        }
    }

    pub fn selected_task(&self) -> Option<&TaskRecord> {
        self.tasks.get(self.selected_task_idx)
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
        self.tasks = store.list_tasks(&TaskFilter::default())?;
        self.leases = store.list_active_leases()?;

        if self.total_items() == 0 {
            self.selected_idx = 0;
        } else if self.selected_idx >= self.total_items() {
            self.selected_idx = self.total_items() - 1;
        }

        if self.tasks.is_empty() {
            self.selected_task_idx = 0;
        } else if self.selected_task_idx >= self.tasks.len() {
            self.selected_task_idx = self.tasks.len() - 1;
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
        assert_eq!(AppTab::from_index(1), AppTab::Swarm);
        assert_eq!(AppTab::from_index(2), AppTab::Memories);
        assert_eq!(AppTab::from_index(3), AppTab::AntiPatterns);
        assert_eq!(AppTab::from_index(4), AppTab::Jtms);
        assert_eq!(AppTab::from_index(5), AppTab::Anchors);
        assert_eq!(AppTab::from_index(99), AppTab::Overview);

        assert_eq!(AppTab::Overview.title(), "Overview");
        assert_eq!(AppTab::Swarm.title(), "Swarm Cockpit");
        assert_eq!(AppTab::Memories.title(), "Memories & Decay");
    }

    #[test]
    fn test_app_tab_cycling() {
        let engine = Arc::new(SqliteMemoryEngine::open_in_memory(None).expect("engine"));
        let mut app = App::new(engine, PathBuf::from("test.db"), "TestWorkspace".to_string())
            .expect("app new");

        assert_eq!(app.active_tab, AppTab::Overview);
        app.next_tab();
        assert_eq!(app.active_tab, AppTab::Swarm);
        app.next_tab();
        assert_eq!(app.active_tab, AppTab::Memories);
        app.prev_tab();
        assert_eq!(app.active_tab, AppTab::Swarm);
        app.prev_tab();
        assert_eq!(app.active_tab, AppTab::Overview);
        app.prev_tab();
        assert_eq!(app.active_tab, AppTab::Anchors);

        app.set_tab(AppTab::Swarm);
        assert_eq!(app.active_tab, AppTab::Swarm);
    }

    #[test]
    fn test_app_swarm_row_navigation() {
        let engine = Arc::new(SqliteMemoryEngine::open_in_memory(None).expect("engine"));
        let mut app = App::new(engine, PathBuf::from("test.db"), "TestWorkspace".to_string())
            .expect("app new");

        assert!(app.tasks.is_empty());
        assert_eq!(app.selected_task(), None);

        // Add dummy tasks to app
        app.tasks.push(TaskRecord::new("task-1", "First Task"));
        app.tasks.push(TaskRecord::new("task-2", "Second Task"));
        app.tasks.push(TaskRecord::new("task-3", "Third Task"));

        app.set_tab(AppTab::Swarm);
        assert_eq!(app.selected_task_idx, 0);
        assert_eq!(app.selected_task().unwrap().task_id, "task-1");

        app.next_row();
        assert_eq!(app.selected_task_idx, 1);
        assert_eq!(app.selected_task().unwrap().task_id, "task-2");

        app.next_row();
        assert_eq!(app.selected_task_idx, 2);
        assert_eq!(app.selected_task().unwrap().task_id, "task-3");

        // Wrap around
        app.next_row();
        assert_eq!(app.selected_task_idx, 0);
        assert_eq!(app.selected_task().unwrap().task_id, "task-1");

        // Prev row wrap backwards
        app.prev_row();
        assert_eq!(app.selected_task_idx, 2);
        assert_eq!(app.selected_task().unwrap().task_id, "task-3");
    }
}
