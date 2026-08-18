use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use serde::{Deserialize, Serialize};
use strata_core::errors::StrataError;
pub use strata_core::schemas::{ContextBudgetConfig, FactStatus, HostTargetConfig};
use strata_core::state::{FailureSeverity, MemoryType};

use crate::store::SqliteStore;

pub const STRATA_MARKER_START: &str = "<!-- STRATA_MEMORY_START -->";
pub const STRATA_MARKER_END: &str = "<!-- STRATA_MEMORY_END -->";

/// Result of compiling instructions for a single host.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HostCompileResult {
    pub host: String,
    pub target_file: PathBuf,
    pub tokens_compiled: usize,
    pub items_count: usize,
    pub updated: bool,
}

/// Overall compilation report across all target hosts.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct MultiHostCompileReport {
    pub target_hosts: Vec<HostCompileResult>,
    pub total_tokens: usize,
    pub budget: usize,
}

/// MultiHostCompiler compiles persistent memory and alignment rules into host instruction files.
pub struct MultiHostCompiler {
    store: Arc<SqliteStore>,
}

impl MultiHostCompiler {
    pub fn new(store: Arc<SqliteStore>) -> Self {
        Self { store }
    }

    /// Compiles context into a unified Markdown block conforming to ContextBudgetConfig.
    pub fn compile_context(&self, config: &ContextBudgetConfig) -> Result<String, StrataError> {
        let max_tokens = if config.max_tokens == 0 { 2048 } else { config.max_tokens };

        // 1. Fetch failure patterns
        let failures = if config.include_failure_patterns {
            self.store.search_failures(None, None, config.top_k_memories.max(10))?
        } else {
            Vec::new()
        };

        // 2. Fetch active semantic facts
        let facts = self.store.get_all_semantic_facts(None, Some(FactStatus::Active), config.top_k_memories.max(20))?;
        // 3. Fetch procedural skills
        let skills = if config.include_success_trajectories {
            self.store.get_all_procedural_skills(None, config.top_k_memories.max(10))?
        } else {
            Vec::new()
        };
        // 4. Fetch general semantic memories
        let memories = self.store.get_all_memories(None, Some(&[MemoryType::Semantic]), config.top_k_memories.max(10))?;

        let mut doc = String::new();
        doc.push_str("## Strata Persistent Memory Protocol\n");
        doc.push_str("- Consult Strata memory tools (`memory_search`, `memory_get`) before planning non-trivial tasks.\n");
        doc.push_str("- Check known failure anti-patterns before running destructive or complex operations.\n");
        doc.push_str("- Record durable takeaways via `memory_write`.\n");

        // Section: Known Failure Anti-Patterns & Mitigations
        if !failures.is_empty() {
            doc.push_str("\n### Known Failure Anti-Patterns\n");
            for f in &failures {
                let severity_tag = match f.severity {
                    FailureSeverity::Critical => "[CRITICAL]",
                    FailureSeverity::High => "[HIGH]",
                    FailureSeverity::Medium => "[MEDIUM]",
                    FailureSeverity::Low => "[LOW]",
                };
                let line = format!(
                    "- {} {}: {}\n  *Mitigation*: {}\n",
                    severity_tag, f.pattern_name, f.description, f.mitigation
                );
                if estimate_tokens(&(doc.clone() + &line)) > max_tokens {
                    break;
                }
                doc.push_str(&line);
            }
        }

        // Section: Verified Semantic Facts & Guidelines
        if !facts.is_empty() || !memories.is_empty() {
            doc.push_str("\n### Verified Semantic Facts\n");
            for f in &facts {
                let line = format!("- [{}] {}\n", f.category, f.statement);
                if estimate_tokens(&(doc.clone() + &line)) > max_tokens {
                    break;
                }
                doc.push_str(&line);
            }

            for m in &memories {
                let title = m.summary.as_deref().unwrap_or(&m.content);
                let line = format!("- {}\n", title);
                if estimate_tokens(&(doc.clone() + &line)) > max_tokens {
                    break;
                }
                doc.push_str(&line);
            }
        }

        // Section: Reusable Procedural Skills
        if !skills.is_empty() {
            doc.push_str("\n### Reusable Procedural Skills\n");
            for s in &skills {
                let line = format!("- **{}**: {}\n", s.name, s.description);
                if estimate_tokens(&(doc.clone() + &line)) > max_tokens {
                    break;
                }
                doc.push_str(&line);
            }
        }

        Ok(doc.trim().to_string())
    }

    /// Sync host instruction files given a ContextBudgetConfig and HostTargetConfig.
    pub fn sync_hosts(
        &self,
        workspace: &Path,
        config: &ContextBudgetConfig,
        hosts: &HostTargetConfig,
    ) -> Result<Vec<String>, StrataError> {
        let compiled_body = self.compile_context(config)?;
        let marked_block = format!("{STRATA_MARKER_START}\n{compiled_body}\n{STRATA_MARKER_END}\n");
        let mut updated_paths = Vec::new();

        if hosts.cursor {
            let cursor_dir = workspace.join(".cursor").join("rules");
            let _ = fs::create_dir_all(&cursor_dir);
            let target_file = cursor_rules_file(workspace, &cursor_dir, &marked_block)?;
            updated_paths.push(target_file.to_string_lossy().to_string());
        }

        if hosts.claude {
            let target_file = workspace.join("CLAUDE.md");
            inject_or_write_file(&target_file, &marked_block, None)?;
            updated_paths.push(target_file.to_string_lossy().to_string());
        }

        if hosts.codex {
            let target_file = workspace.join("AGENTS.md");
            inject_or_write_file(&target_file, &marked_block, None)?;
            updated_paths.push(target_file.to_string_lossy().to_string());
        }

        if hosts.gemini {
            let gemini_dir = workspace.join(".gemini");
            let _ = fs::create_dir_all(&gemini_dir);
            let target_file = gemini_dir.join("GEMINI.md");
            inject_or_write_file(&target_file, &marked_block, None)?;
            updated_paths.push(target_file.to_string_lossy().to_string());
        }

        Ok(updated_paths)
    }

    /// Compiles and synchronizes instruction files across all specified host targets.
    pub fn compile_workspace(
        &self,
        workspace: &Path,
        targets: &[&str],
        budget: usize,
    ) -> Result<MultiHostCompileReport, StrataError> {
        let config = ContextBudgetConfig::new(budget, 50);
        let compiled_body = self.compile_context(&config)?;
        let token_count = estimate_tokens(&compiled_body);
        let marked_block = format!("{STRATA_MARKER_START}\n{compiled_body}\n{STRATA_MARKER_END}\n");

        let mut report = MultiHostCompileReport {
            target_hosts: Vec::new(),
            total_tokens: token_count,
            budget,
        };

        let target_list: Vec<&str> = if targets.contains(&"all") || targets.is_empty() {
            vec!["cursor", "claude", "codex", "gemini"]
        } else {
            targets.to_vec()
        };

        for &host in &target_list {
            let host_clean = host.trim().to_lowercase();
            match host_clean.as_str() {
                "cursor" => {
                    let cursor_dir = workspace.join(".cursor").join("rules");
                    let _ = fs::create_dir_all(&cursor_dir);
                    let target_file = cursor_rules_file(workspace, &cursor_dir, &marked_block)?;

                    report.target_hosts.push(HostCompileResult {
                        host: "cursor".to_string(),
                        target_file,
                        tokens_compiled: token_count,
                        items_count: 1,
                        updated: true,
                    });
                }
                "claude" | "claude-code" => {
                    let target_file = workspace.join("CLAUDE.md");
                    inject_or_write_file(&target_file, &marked_block, None)?;

                    report.target_hosts.push(HostCompileResult {
                        host: "claude".to_string(),
                        target_file,
                        tokens_compiled: token_count,
                        items_count: 1,
                        updated: true,
                    });
                }
                "codex" => {
                    let target_file = workspace.join("AGENTS.md");
                    inject_or_write_file(&target_file, &marked_block, None)?;

                    report.target_hosts.push(HostCompileResult {
                        host: "codex".to_string(),
                        target_file,
                        tokens_compiled: token_count,
                        items_count: 1,
                        updated: true,
                    });
                }
                "gemini" | "antigravity" => {
                    let gemini_dir = workspace.join(".gemini");
                    let _ = fs::create_dir_all(&gemini_dir);
                    let target_file = gemini_dir.join("GEMINI.md");
                    inject_or_write_file(&target_file, &marked_block, None)?;

                    report.target_hosts.push(HostCompileResult {
                        host: "gemini".to_string(),
                        target_file,
                        tokens_compiled: token_count,
                        items_count: 1,
                        updated: true,
                    });
                }
                _ => {}
            }
        }

        Ok(report)
    }
}

fn cursor_rules_file(_workspace: &Path, cursor_dir: &Path, marked_block: &str) -> Result<PathBuf, StrataError> {
    let target_file = cursor_dir.join("strata.mdc");
    let default_cursor = format!(
        "---\ndescription: Strata Persistent Cognitive Memory & Alignment Context\nglobs: *\nalwaysApply: true\n---\n{marked_block}"
    );

    if target_file.exists() {
        let existing = fs::read_to_string(&target_file).map_err(|e| StrataError::Io(e.to_string()))?;
        if existing.contains(STRATA_MARKER_START) {
            inject_or_write_file(&target_file, marked_block, None)?;
        } else if existing.starts_with("---") {
            // Frontmatter exists, insert markers after frontmatter
            if let Some(second_dash) = existing[3..].find("---") {
                let end_fm = 3 + second_dash + 3;
                let fm = &existing[..end_fm];
                let rest = &existing[end_fm..];
                let new_content = format!("{fm}\n{marked_block}\n{rest}");
                fs::write(&target_file, new_content.trim_start()).map_err(|e| StrataError::Io(e.to_string()))?;
            } else {
                inject_or_write_file(&target_file, marked_block, Some(&default_cursor))?;
            }
        } else {
            inject_or_write_file(&target_file, marked_block, Some(&default_cursor))?;
        }
    } else {
        fs::write(&target_file, default_cursor).map_err(|e| StrataError::Io(e.to_string()))?;
    }

    Ok(target_file)
}

/// Helper to inject or replace marked content block in a target instruction file.
fn inject_or_write_file(path: &Path, marked_block: &str, default_full_content: Option<&str>) -> Result<(), StrataError> {
    let existing = if path.exists() {
        fs::read_to_string(path).map_err(|e| StrataError::Io(e.to_string()))?
    } else {
        String::new()
    };

    if let (Some(start_idx), Some(end_idx)) = (
        existing.find(STRATA_MARKER_START),
        existing.find(STRATA_MARKER_END),
    ) {
        let before = &existing[..start_idx];
        let after = &existing[end_idx + STRATA_MARKER_END.len()..];
        let new_content = format!("{before}{marked_block}{after}");
        fs::write(path, new_content.trim_start()).map_err(|e| StrataError::Io(e.to_string()))?;
    } else if existing.trim().is_empty() {
        let content_to_write = default_full_content.unwrap_or(marked_block);
        fs::write(path, content_to_write).map_err(|e| StrataError::Io(e.to_string()))?;
    } else {
        let new_content = format!("{}\n\n{}", marked_block.trim(), existing.trim());
        fs::write(path, new_content).map_err(|e| StrataError::Io(e.to_string()))?;
    }

    Ok(())
}

/// Estimate tokens from character count (~3.5 to 4 characters per token).
pub fn estimate_tokens(text: &str) -> usize {
    (text.chars().count() + 3) / 4
}
