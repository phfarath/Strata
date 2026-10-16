use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use strata_core::errors::StrataError;
pub use strata_core::schemas::{ContextBudgetConfig, FactStatus, HostTargetConfig};
use strata_core::state::{FailureSeverity, MemoryType};

use crate::store::SqliteStore;

pub const STRATA_MARKER_START: &str = "<!-- STRATA_MEMORY_START -->";
pub const STRATA_MARKER_END: &str = "<!-- STRATA_MEMORY_END -->";

const PATH_GLOB_TAG_PREFIX: &str = "path_glob:";

/// Extension trait for attaching and querying path-scoped glob patterns across memory and rule records.
pub trait PathScopedExt {
    fn with_path_glob(self, glob: impl Into<String>) -> Self;
    fn path_glob(&self) -> Option<String>;

    fn matches_active_file(&self, active_file: Option<&str>) -> bool {
        is_eligible_for_file(self.path_glob().as_deref(), active_file)
    }
}

/// A Core Tier architectural or project rule that can optionally be scoped to a path glob.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoreRule {
    pub id: String,
    pub title: String,
    pub content: String,
    #[serde(default)]
    pub path_glob: Option<String>,
    #[serde(default = "default_rule_priority")]
    pub priority: u32,
}

fn default_rule_priority() -> u32 {
    100
}

impl CoreRule {
    pub fn new(
        id: impl Into<String>,
        title: impl Into<String>,
        content: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            content: content.into(),
            path_glob: None,
            priority: default_rule_priority(),
        }
    }

    pub fn with_path_glob(mut self, glob: impl Into<String>) -> Self {
        let g = glob.into();
        if g.trim().is_empty() {
            self.path_glob = None;
        } else {
            self.path_glob = Some(g);
        }
        self
    }

    pub fn with_priority(mut self, priority: u32) -> Self {
        self.priority = priority;
        self
    }
}

impl PathScopedExt for CoreRule {
    fn with_path_glob(self, glob: impl Into<String>) -> Self {
        CoreRule::with_path_glob(self, glob)
    }

    fn path_glob(&self) -> Option<String> {
        self.path_glob.clone()
    }
}

/// Semantic memory representation with first-class `path_glob` support and lossless conversion
/// to/from `MemoryRecord` and `SemanticFact`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SemanticMemory {
    pub id: uuid::Uuid,
    pub statement: String,
    pub category: String,
    pub scope: strata_core::state::Scope,
    pub importance: f32,
    pub confidence: f32,
    #[serde(default)]
    pub path_glob: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
}

impl SemanticMemory {
    pub fn new(
        statement: impl Into<String>,
        category: impl Into<String>,
        scope: strata_core::state::Scope,
    ) -> Self {
        Self {
            id: uuid::Uuid::new_v4(),
            statement: statement.into(),
            category: category.into(),
            scope,
            importance: 0.5,
            confidence: 1.0,
            path_glob: None,
            tags: Vec::new(),
        }
    }

    pub fn with_id(mut self, id: uuid::Uuid) -> Self {
        self.id = id;
        self
    }

    pub fn with_path_glob(mut self, glob: impl Into<String>) -> Self {
        let g = glob.into();
        if g.trim().is_empty() {
            self.path_glob = None;
        } else {
            self.path_glob = Some(g);
        }
        self
    }

    pub fn with_importance(mut self, importance: f32) -> Self {
        self.importance = importance.clamp(0.0, 1.0);
        self
    }

    pub fn with_confidence(mut self, confidence: f32) -> Self {
        self.confidence = confidence.clamp(0.0, 1.0);
        self
    }

    pub fn with_tags(mut self, tags: Vec<String>) -> Self {
        self.tags = tags;
        self
    }

    pub fn to_memory_record(&self) -> strata_core::state::MemoryRecord {
        strata_core::state::MemoryRecord::from(self)
    }

    pub fn from_memory_record(record: &strata_core::state::MemoryRecord) -> Self {
        Self::from(record)
    }

    pub fn to_semantic_fact(&self) -> strata_core::schemas::SemanticFact {
        strata_core::schemas::SemanticFact::from(self)
    }

    pub fn from_semantic_fact(fact: &strata_core::schemas::SemanticFact) -> Self {
        Self::from(fact)
    }
}

impl PathScopedExt for SemanticMemory {
    fn with_path_glob(self, glob: impl Into<String>) -> Self {
        SemanticMemory::with_path_glob(self, glob)
    }

    fn path_glob(&self) -> Option<String> {
        self.path_glob.clone()
    }
}

impl PathScopedExt for strata_core::state::MemoryRecord {
    fn with_path_glob(mut self, glob: impl Into<String>) -> Self {
        let g = glob.into();
        let mut map = match self.metadata {
            serde_json::Value::Object(m) => m,
            _ => serde_json::Map::new(),
        };
        if g.trim().is_empty() {
            map.remove("path_glob");
        } else {
            map.insert("path_glob".to_string(), serde_json::Value::String(g));
        }
        self.metadata = serde_json::Value::Object(map);
        self
    }

    fn path_glob(&self) -> Option<String> {
        self.metadata
            .get("path_glob")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .or_else(|| {
                self.tags
                    .iter()
                    .find_map(|t| t.strip_prefix(PATH_GLOB_TAG_PREFIX))
                    .map(|s| s.to_string())
            })
            .filter(|s| !s.trim().is_empty())
    }
}

impl PathScopedExt for strata_core::state::FailurePattern {
    fn with_path_glob(mut self, glob: impl Into<String>) -> Self {
        let g = glob.into();
        let mut map = match self.metadata {
            serde_json::Value::Object(m) => m,
            _ => serde_json::Map::new(),
        };
        if g.trim().is_empty() {
            map.remove("path_glob");
        } else {
            map.insert("path_glob".to_string(), serde_json::Value::String(g));
        }
        self.metadata = serde_json::Value::Object(map);
        self
    }

    fn path_glob(&self) -> Option<String> {
        self.metadata
            .get("path_glob")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .filter(|s| !s.trim().is_empty())
    }
}

impl PathScopedExt for strata_core::schemas::SemanticFact {
    fn with_path_glob(mut self, glob: impl Into<String>) -> Self {
        let g = glob.into();
        self.tags
            .retain(|t| !t.starts_with(PATH_GLOB_TAG_PREFIX));
        if !g.trim().is_empty() {
            self.tags.push(format!("{PATH_GLOB_TAG_PREFIX}{g}"));
        }
        self
    }

    fn path_glob(&self) -> Option<String> {
        self.tags
            .iter()
            .find_map(|t| t.strip_prefix(PATH_GLOB_TAG_PREFIX))
            .map(|s| s.to_string())
            .filter(|s| !s.trim().is_empty())
    }
}

impl From<&SemanticMemory> for strata_core::state::MemoryRecord {
    fn from(mem: &SemanticMemory) -> Self {
        let mut record = strata_core::state::MemoryRecord::new(
            MemoryType::Semantic,
            mem.statement.clone(),
            mem.scope.clone(),
        )
        .with_importance(mem.importance)
        .with_confidence(mem.confidence)
        .with_tags(mem.tags.clone());
        record.id = mem.id;

        let mut map = serde_json::Map::new();
        map.insert(
            "category".to_string(),
            serde_json::Value::String(mem.category.clone()),
        );
        if let Some(ref glob) = mem.path_glob {
            map.insert(
                "path_glob".to_string(),
                serde_json::Value::String(glob.clone()),
            );
        }
        record.metadata = serde_json::Value::Object(map);
        record
    }
}

impl From<SemanticMemory> for strata_core::state::MemoryRecord {
    fn from(mem: SemanticMemory) -> Self {
        Self::from(&mem)
    }
}

impl From<&strata_core::state::MemoryRecord> for SemanticMemory {
    fn from(record: &strata_core::state::MemoryRecord) -> Self {
        let category = record
            .metadata
            .get("category")
            .and_then(|v| v.as_str())
            .unwrap_or("general")
            .to_string();
        let clean_tags: Vec<String> = record
            .tags
            .iter()
            .filter(|t| !t.starts_with(PATH_GLOB_TAG_PREFIX))
            .cloned()
            .collect();
        Self {
            id: record.id,
            statement: record.content.clone(),
            category,
            scope: record.scope.clone(),
            importance: record.importance,
            confidence: record.confidence,
            path_glob: record.path_glob(),
            tags: clean_tags,
        }
    }
}

impl From<strata_core::state::MemoryRecord> for SemanticMemory {
    fn from(record: strata_core::state::MemoryRecord) -> Self {
        Self::from(&record)
    }
}

impl From<&SemanticMemory> for strata_core::schemas::SemanticFact {
    fn from(mem: &SemanticMemory) -> Self {
        let mut fact = strata_core::schemas::SemanticFact::new(
            mem.statement.clone(),
            mem.category.clone(),
            mem.scope.clone(),
        )
        .with_id(mem.id)
        .with_importance(mem.importance)
        .with_confidence(mem.confidence)
        .with_tags(mem.tags.clone());
        if let Some(ref glob) = mem.path_glob {
            fact = fact.with_path_glob(glob.clone());
        }
        fact
    }
}

impl From<SemanticMemory> for strata_core::schemas::SemanticFact {
    fn from(mem: SemanticMemory) -> Self {
        Self::from(&mem)
    }
}

impl From<&strata_core::schemas::SemanticFact> for SemanticMemory {
    fn from(fact: &strata_core::schemas::SemanticFact) -> Self {
        let clean_tags: Vec<String> = fact
            .tags
            .iter()
            .filter(|t| !t.starts_with(PATH_GLOB_TAG_PREFIX))
            .cloned()
            .collect();
        Self {
            id: fact.id,
            statement: fact.statement.clone(),
            category: fact.category.clone(),
            scope: fact.scope.clone(),
            importance: fact.importance,
            confidence: fact.confidence,
            path_glob: fact.path_glob(),
            tags: clean_tags,
        }
    }
}

impl From<strata_core::schemas::SemanticFact> for SemanticMemory {
    fn from(fact: strata_core::schemas::SemanticFact) -> Self {
        Self::from(&fact)
    }
}

fn is_global_glob(glob: &str) -> bool {
    let trimmed = glob.trim();
    trimmed.is_empty() || trimmed == "*" || trimmed == "**" || trimmed == "**/*"
}

fn is_eligible_for_file(path_glob: Option<&str>, active_file: Option<&str>) -> bool {
    match path_glob {
        None => true,
        Some(glob) if is_global_glob(glob) => true,
        Some(glob) => match active_file {
            Some(file) if !file.trim().is_empty() => matches_path_glob(glob, file),
            _ => false,
        },
    }
}

/// Deterministic glob matcher supporting `*`, `**`, `?`, and `{a,b}` brace expansion.
pub fn matches_path_glob(pattern: &str, file_path: &str) -> bool {
    let norm_path = normalize_glob_path(file_path);
    if norm_path.is_empty() {
        return false;
    }

    for sub_pat in split_top_level_commas(pattern) {
        for expanded in expand_braces(&sub_pat) {
            let norm_pat = normalize_glob_path(&expanded);
            if norm_pat.is_empty() {
                continue;
            }
            if is_global_glob(&norm_pat) {
                return true;
            }
            if !norm_pat.contains('/') {
                let basename = norm_path.rsplit('/').next().unwrap_or(&norm_path);
                if match_segment(&norm_pat, &norm_path) || match_segment(&norm_pat, basename) {
                    return true;
                }
            } else {
                let pat_segs: Vec<&str> = norm_pat.split('/').collect();
                let path_segs: Vec<&str> = norm_path.split('/').collect();
                if match_segments(&pat_segs, &path_segs) {
                    return true;
                }
            }
        }
    }

    false
}

fn normalize_glob_path(p: &str) -> String {
    let replaced = p.trim().replace('\\', "/");
    let stripped = replaced.trim_start_matches("./").trim_start_matches('/');
    let mut out = String::with_capacity(stripped.len());
    let mut prev_slash = false;
    for ch in stripped.chars() {
        if ch == '/' {
            if !prev_slash {
                out.push(ch);
            }
            prev_slash = true;
        } else {
            out.push(ch);
            prev_slash = false;
        }
    }
    out
}

fn split_top_level_commas(pattern: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut depth = 0usize;
    let mut last = 0usize;
    for (idx, ch) in pattern.char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                let slice = pattern[last..idx].trim();
                if !slice.is_empty() {
                    parts.push(slice.to_string());
                }
                last = idx + 1;
            }
            _ => {}
        }
    }
    let tail = pattern[last..].trim();
    if !tail.is_empty() {
        parts.push(tail.to_string());
    }
    parts
}

fn expand_braces(pattern: &str) -> Vec<String> {
    if let Some(open_idx) = pattern.find('{') {
        if let Some(rel_close) = pattern[open_idx + 1..].find('}') {
            let close_idx = open_idx + 1 + rel_close;
            let prefix = &pattern[..open_idx];
            let inner = &pattern[open_idx + 1..close_idx];
            let suffix = &pattern[close_idx + 1..];
            let mut results = Vec::new();
            for alt in inner.split(',') {
                let combined = format!("{}{}{}", prefix, alt.trim(), suffix);
                results.extend(expand_braces(&combined));
            }
            return results;
        }
    }
    vec![pattern.to_string()]
}

fn match_segments(pat_segs: &[&str], path_segs: &[&str]) -> bool {
    if pat_segs.is_empty() {
        return path_segs.is_empty();
    }

    if pat_segs[0] == "**" {
        let mut next_pat = &pat_segs[1..];
        while !next_pat.is_empty() && next_pat[0] == "**" {
            next_pat = &next_pat[1..];
        }
        if next_pat.is_empty() {
            return true;
        }
        if match_segments(next_pat, path_segs) {
            return true;
        }
        if !path_segs.is_empty() {
            return match_segments(pat_segs, &path_segs[1..]);
        }
        return false;
    }

    if path_segs.is_empty() {
        return false;
    }

    match_segment(pat_segs[0], path_segs[0]) && match_segments(&pat_segs[1..], &path_segs[1..])
}

fn match_segment(pat: &str, text: &str) -> bool {
    let p_chars: Vec<char> = pat.chars().collect();
    let t_chars: Vec<char> = text.chars().collect();
    match_chars(&p_chars, &t_chars)
}

fn match_chars(pat: &[char], text: &[char]) -> bool {
    let (mut p_idx, mut t_idx) = (0usize, 0usize);
    let mut star_idx: Option<usize> = None;
    let mut match_idx = 0usize;

    while t_idx < text.len() {
        if p_idx < pat.len() && (pat[p_idx] == '?' || pat[p_idx] == text[t_idx]) {
            p_idx += 1;
            t_idx += 1;
        } else if p_idx < pat.len() && pat[p_idx] == '*' {
            star_idx = Some(p_idx);
            match_idx = t_idx;
            p_idx += 1;
        } else if let Some(s_idx) = star_idx {
            p_idx = s_idx + 1;
            match_idx += 1;
            t_idx = match_idx;
        } else {
            return false;
        }
    }

    while p_idx < pat.len() && pat[p_idx] == '*' {
        p_idx += 1;
    }

    p_idx == pat.len()
}

/// Derives a deterministic file slug for a path glob pattern (e.g. `src/frontend/**/*.tsx` -> `strata-scoped-src-frontend-tsx`).
pub fn glob_to_slug(glob: &str) -> String {
    let trimmed = glob.trim();
    if let Some(rest) = trimmed.strip_prefix("strata-scoped-") {
        let inner = glob_to_slug(rest);
        return inner;
    }
    let tokens: Vec<String> = trimmed
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_ascii_lowercase())
        .collect();
    if tokens.is_empty() {
        "strata-scoped-all".to_string()
    } else {
        format!("strata-scoped-{}", tokens.join("-"))
    }
}

#[derive(Default)]
struct ScopedGroup {
    core_rules: Vec<CoreRule>,
    failures: Vec<strata_core::state::FailurePattern>,
    facts: Vec<strata_core::schemas::SemanticFact>,
    memories: Vec<strata_core::state::MemoryRecord>,
}

impl ScopedGroup {
    fn total_items(&self) -> usize {
        self.core_rules.len() + self.failures.len() + self.facts.len() + self.memories.len()
    }
}

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
        self.compile_context_for_file(config, None, &[])
    }

    /// Compiles context filtered by an optional `active_file` and additional `core_rules`.
    /// Path-scoped items are only injected when `active_file` matches their `path_glob`.
    pub fn compile_context_for_file(
        &self,
        config: &ContextBudgetConfig,
        active_file: Option<&str>,
        core_rules: &[CoreRule],
    ) -> Result<String, StrataError> {
        let max_tokens = if config.max_tokens == 0 {
            2048
        } else {
            config.max_tokens
        };

        // Filter core rules by active_file
        let mut eligible_rules: Vec<&CoreRule> = core_rules
            .iter()
            .filter(|r| r.matches_active_file(active_file))
            .collect();
        eligible_rules.sort_by(|a, b| b.priority.cmp(&a.priority).then_with(|| a.id.cmp(&b.id)));

        // 1. Fetch and filter failure patterns
        let failures: Vec<_> = if config.include_failure_patterns {
            self.store
                .search_failures(None, None, config.top_k_memories.max(10))?
                .into_iter()
                .filter(|f| f.matches_active_file(active_file))
                .collect()
        } else {
            Vec::new()
        };

        // 2. Fetch and filter active semantic facts
        let facts: Vec<_> = self
            .store
            .get_all_semantic_facts(
                None,
                Some(FactStatus::Active),
                config.top_k_memories.max(20),
            )?
            .into_iter()
            .filter(|f| f.matches_active_file(active_file))
            .collect();

        // 3. Fetch procedural skills
        let skills = if config.include_success_trajectories {
            self.store
                .get_all_procedural_skills(None, config.top_k_memories.max(10))?
        } else {
            Vec::new()
        };

        // 4. Fetch and filter general semantic memories
        let memories: Vec<_> = self
            .store
            .get_all_memories(
                None,
                Some(&[MemoryType::Semantic]),
                config.top_k_memories.max(10),
            )?
            .into_iter()
            .filter(|m| m.matches_active_file(active_file))
            .collect();

        let mut doc = String::new();
        doc.push_str("## Strata Persistent Memory Protocol\n");
        doc.push_str("- Consult Strata memory tools (`memory_search`, `memory_get`) before planning non-trivial tasks.\n");
        doc.push_str("- Check known failure anti-patterns before running destructive or complex operations.\n");
        doc.push_str("- Record durable takeaways via `memory_write`.\n");

        // Section: Core Rules (if any eligible)
        if !eligible_rules.is_empty() {
            doc.push_str("\n### Core Rules\n");
            for r in &eligible_rules {
                let line = if r.title.trim().is_empty() {
                    format!("- {}\n", r.content)
                } else {
                    format!("- **{}**: {}\n", r.title, r.content)
                };
                if estimate_tokens(&(doc.clone() + &line)) > max_tokens {
                    break;
                }
                doc.push_str(&line);
            }
        }

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

    /// Compiles path-scoped rules into host-native scoped rule files (`.cursor/rules/<slug>.mdc` and `.claude/rules/<slug>.md`).
    pub fn compile_scoped_rules(
        &self,
        workspace: &Path,
        targets: &[&str],
        core_rules: &[CoreRule],
        budget: usize,
    ) -> Result<MultiHostCompileReport, StrataError> {
        let max_tokens = if budget == 0 { 2048 } else { budget };
        let failures = self.store.search_failures(None, None, 100)?;
        let facts = self
            .store
            .get_all_semantic_facts(None, Some(FactStatus::Active), 100)?;
        let memories = self
            .store
            .get_all_memories(None, Some(&[MemoryType::Semantic]), 100)?;

        let mut groups: std::collections::BTreeMap<String, ScopedGroup> =
            std::collections::BTreeMap::new();

        for r in core_rules {
            if let Some(glob) = r.path_glob() {
                if !is_global_glob(&glob) {
                    groups.entry(glob).or_default().core_rules.push(r.clone());
                }
            }
        }

        for f in failures {
            if let Some(glob) = f.path_glob() {
                if !is_global_glob(&glob) {
                    groups.entry(glob).or_default().failures.push(f);
                }
            }
        }

        for f in facts {
            if let Some(glob) = f.path_glob() {
                if !is_global_glob(&glob) {
                    groups.entry(glob).or_default().facts.push(f);
                }
            }
        }

        for m in memories {
            if let Some(glob) = m.path_glob() {
                if !is_global_glob(&glob) {
                    groups.entry(glob).or_default().memories.push(m);
                }
            }
        }

        let target_list: Vec<&str> = if targets.contains(&"all") || targets.is_empty() {
            vec!["cursor", "claude", "codex", "gemini"]
        } else {
            targets.to_vec()
        };

        let mut report = MultiHostCompileReport {
            target_hosts: Vec::new(),
            total_tokens: 0,
            budget,
        };

        for (glob, mut group) in groups {
            let items_count = group.total_items();
            if items_count == 0 {
                continue;
            }

            group
                .core_rules
                .sort_by(|a, b| b.priority.cmp(&a.priority).then_with(|| a.id.cmp(&b.id)));

            let mut doc = format!("## Strata Scoped Rules ({glob})\n");

            if !group.core_rules.is_empty() {
                doc.push_str("\n### Core Rules\n");
                for r in &group.core_rules {
                    let line = if r.title.trim().is_empty() {
                        format!("- {}\n", r.content)
                    } else {
                        format!("- **{}**: {}\n", r.title, r.content)
                    };
                    if estimate_tokens(&(doc.clone() + &line)) > max_tokens {
                        break;
                    }
                    doc.push_str(&line);
                }
            }

            if !group.failures.is_empty() {
                doc.push_str("\n### Known Failure Anti-Patterns\n");
                for f in &group.failures {
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

            if !group.facts.is_empty() || !group.memories.is_empty() {
                doc.push_str("\n### Verified Semantic Facts\n");
                for f in &group.facts {
                    let line = format!("- [{}] {}\n", f.category, f.statement);
                    if estimate_tokens(&(doc.clone() + &line)) > max_tokens {
                        break;
                    }
                    doc.push_str(&line);
                }
                for m in &group.memories {
                    let title = m.summary.as_deref().unwrap_or(&m.content);
                    let line = format!("- {}\n", title);
                    if estimate_tokens(&(doc.clone() + &line)) > max_tokens {
                        break;
                    }
                    doc.push_str(&line);
                }
            }

            let compiled_body = doc.trim().to_string();
            let token_count = estimate_tokens(&compiled_body);
            let marked_block =
                format!("{STRATA_MARKER_START}\n{compiled_body}\n{STRATA_MARKER_END}\n");
            let slug = glob_to_slug(&glob);

            for &host in &target_list {
                let host_clean = host.trim().to_lowercase();
                match host_clean.as_str() {
                    "cursor" => {
                        let cursor_dir = workspace.join(".cursor").join("rules");
                        let _ = fs::create_dir_all(&cursor_dir);
                        let target_file = cursor_dir.join(format!("{slug}.mdc"));
                        let default_cursor = format!(
                            "---\ndescription: Strata Scoped Rules ({glob})\nglobs: {glob}\nalwaysApply: false\n---\n{marked_block}"
                        );
                        inject_or_write_file(&target_file, &marked_block, Some(&default_cursor))?;
                        report.target_hosts.push(HostCompileResult {
                            host: "cursor".to_string(),
                            target_file,
                            tokens_compiled: token_count,
                            items_count,
                            updated: true,
                        });
                        report.total_tokens += token_count;
                    }
                    "claude" | "claude-code" => {
                        let claude_rules_dir = workspace.join(".claude").join("rules");
                        let _ = fs::create_dir_all(&claude_rules_dir);
                        let target_file = claude_rules_dir.join(format!("{slug}.md"));
                        let default_claude = format!("---\npaths: {glob}\n---\n{marked_block}");
                        inject_or_write_file(&target_file, &marked_block, Some(&default_claude))?;
                        report.target_hosts.push(HostCompileResult {
                            host: "claude".to_string(),
                            target_file,
                            tokens_compiled: token_count,
                            items_count,
                            updated: true,
                        });
                        report.total_tokens += token_count;
                    }
                    _ => {}
                }
            }
        }

        Ok(report)
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
        let mut scoped_targets = Vec::new();

        if hosts.cursor {
            let cursor_dir = workspace.join(".cursor").join("rules");
            let _ = fs::create_dir_all(&cursor_dir);
            let target_file = cursor_rules_file(workspace, &cursor_dir, &marked_block)?;
            updated_paths.push(target_file.to_string_lossy().to_string());
            scoped_targets.push("cursor");
        }

        if hosts.claude {
            let target_file = workspace.join("CLAUDE.md");
            inject_or_write_file(&target_file, &marked_block, None)?;
            updated_paths.push(target_file.to_string_lossy().to_string());
            scoped_targets.push("claude");
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

        if !scoped_targets.is_empty() {
            let scoped_report =
                self.compile_scoped_rules(workspace, &scoped_targets, &[], config.max_tokens)?;
            for res in scoped_report.target_hosts {
                updated_paths.push(res.target_file.to_string_lossy().to_string());
            }
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
        self.compile_workspace_for_file(workspace, targets, budget, None, &[])
    }

    /// Compiles and synchronizes instruction files across all specified host targets, optionally filtering for an active file.
    pub fn compile_workspace_for_file(
        &self,
        workspace: &Path,
        targets: &[&str],
        budget: usize,
        active_file: Option<&str>,
        core_rules: &[CoreRule],
    ) -> Result<MultiHostCompileReport, StrataError> {
        let config = ContextBudgetConfig::new(budget, 50);
        let compiled_body = self.compile_context_for_file(&config, active_file, core_rules)?;
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

        let scoped_report =
            self.compile_scoped_rules(workspace, &target_list, core_rules, budget)?;
        report.total_tokens += scoped_report.total_tokens;
        report.target_hosts.extend(scoped_report.target_hosts);

        Ok(report)
    }
}

fn cursor_rules_file(
    _workspace: &Path,
    cursor_dir: &Path,
    marked_block: &str,
) -> Result<PathBuf, StrataError> {
    let target_file = cursor_dir.join("strata.mdc");
    let default_cursor = format!(
        "---\ndescription: Strata Persistent Cognitive Memory & Alignment Context\nglobs: *\nalwaysApply: true\n---\n{marked_block}"
    );

    if target_file.exists() {
        let existing =
            fs::read_to_string(&target_file).map_err(|e| StrataError::Io(e.to_string()))?;
        if existing.contains(STRATA_MARKER_START) {
            inject_or_write_file(&target_file, marked_block, None)?;
        } else if existing.starts_with("---") {
            // Frontmatter exists, insert markers after frontmatter
            if let Some(second_dash) = existing[3..].find("---") {
                let end_fm = 3 + second_dash + 3;
                let fm = &existing[..end_fm];
                let rest = &existing[end_fm..];
                let new_content = format!("{fm}\n{marked_block}\n{rest}");
                fs::write(&target_file, new_content.trim_start())
                    .map_err(|e| StrataError::Io(e.to_string()))?;
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
fn inject_or_write_file(
    path: &Path,
    marked_block: &str,
    default_full_content: Option<&str>,
) -> Result<(), StrataError> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use strata_core::schemas::SemanticFact;
    use strata_core::state::{FailurePattern, Scope};
    use uuid::Uuid;

    #[test]
    fn test_path_glob_roundtrip_sqlite() {
        let store = SqliteStore::open_in_memory().expect("open in memory");

        let rule = CoreRule::new(
            "rust-no-unwrap",
            "No unwrap in library crates",
            "Use ? or map_err instead of .unwrap() in production code",
        )
        .with_path_glob("crates/**/*.rs")
        .with_priority(90);
        assert_eq!(rule.path_glob().as_deref(), Some("crates/**/*.rs"));

        let sem_mem = SemanticMemory::new(
            "React components must use Tailwind utility classes",
            "frontend_convention",
            Scope::Global,
        )
        .with_path_glob("web/src/**/*.tsx")
        .with_tags(vec!["react".to_string(), "ui".to_string()]);
        assert_eq!(sem_mem.path_glob().as_deref(), Some("web/src/**/*.tsx"));

        // Roundtrip via MemoryRecord in SqliteStore
        let record = sem_mem.to_memory_record();
        assert_eq!(record.path_glob().as_deref(), Some("web/src/**/*.tsx"));
        store
            .insert_or_update_memory(&record)
            .expect("insert memory");
        let loaded_rec = store
            .get_memory(&record.id)
            .expect("get memory")
            .expect("exists");
        assert_eq!(loaded_rec.path_glob().as_deref(), Some("web/src/**/*.tsx"));
        let roundtrip_mem = SemanticMemory::from_memory_record(&loaded_rec);
        assert_eq!(roundtrip_mem.path_glob.as_deref(), Some("web/src/**/*.tsx"));
        assert_eq!(roundtrip_mem.statement, sem_mem.statement);
        assert_eq!(roundtrip_mem.tags, vec!["react".to_string(), "ui".to_string()]);

        // Roundtrip via SemanticFact in SqliteStore
        let fact = sem_mem.to_semantic_fact();
        assert_eq!(fact.path_glob().as_deref(), Some("web/src/**/*.tsx"));
        store
            .insert_or_update_semantic_fact(&fact)
            .expect("insert fact");
        let loaded_fact = store
            .get_semantic_fact(&fact.id)
            .expect("get fact")
            .expect("exists");
        assert_eq!(
            loaded_fact.path_glob().as_deref(),
            Some("web/src/**/*.tsx")
        );
        let roundtrip_from_fact = SemanticMemory::from_semantic_fact(&loaded_fact);
        assert_eq!(
            roundtrip_from_fact.path_glob.as_deref(),
            Some("web/src/**/*.tsx")
        );
        assert_eq!(
            roundtrip_from_fact.tags,
            vec!["react".to_string(), "ui".to_string()]
        );
    }

    #[test]
    fn test_matches_path_glob_patterns() {
        // Nested ** and zero-segment **
        assert!(matches_path_glob("src/**/*.ts", "src/index.ts"));
        assert!(matches_path_glob("src/**/*.ts", "src/a/b/index.ts"));
        assert!(matches_path_glob(
            "crates/**/*.rs",
            "crates\\strata-memory\\src\\compiler.rs"
        ));
        assert!(!matches_path_glob(
            "crates/**/*.rs",
            "web/src/components/App.tsx"
        ));

        // Extensions and single-segment *
        assert!(matches_path_glob("*.rs", "lib.rs"));
        assert!(matches_path_glob("*.rs", "crates/strata-core/src/lib.rs"));
        assert!(!matches_path_glob("*.rs", "web/App.tsx"));
        assert!(matches_path_glob("src/*.rs", "src/lib.rs"));
        assert!(!matches_path_glob("src/*.rs", "src/nested/lib.rs"));

        // Brace expansion
        assert!(matches_path_glob(
            "web/src/**/*.{ts,tsx}",
            "web/src/components/App.tsx"
        ));
        assert!(matches_path_glob(
            "web/src/**/*.{ts,tsx}",
            "web/src/hooks/useAuth.ts"
        ));
        assert!(!matches_path_glob(
            "web/src/**/*.{ts,tsx}",
            "web/src/styles/main.css"
        ));

        // Deterministic slug generation
        assert_eq!(
            glob_to_slug("src/frontend/**/*.tsx"),
            "strata-scoped-src-frontend-tsx"
        );
    }

    #[test]
    fn test_active_file_filtering_prevents_irrelevant_rule_injection() {
        let store = Arc::new(SqliteStore::open_in_memory().expect("open store"));

        let global_fact = SemanticFact::new(
            "Always run tests before committing",
            "workflow",
            Scope::Global,
        );
        let backend_fact = SemanticFact::new(
            "Use rusqlite with WAL mode for concurrent reads",
            "backend",
            Scope::Global,
        )
        .with_path_glob("crates/**/*.rs");
        let frontend_mem = SemanticMemory::new(
            "Use React functional components with hooks",
            "frontend",
            Scope::Global,
        )
        .with_path_glob("web/src/**/*.tsx")
        .to_memory_record();
        let backend_failure = FailurePattern::new(
            "sig-unwrap-panic",
            "UnwrapPanicInProd",
            "Calling unwrap on Result in Rust crate",
            "Propagate error via ? operator",
        )
        .with_path_glob("crates/**/*.rs");

        store
            .insert_or_update_semantic_fact(&global_fact)
            .expect("insert global fact");
        store
            .insert_or_update_semantic_fact(&backend_fact)
            .expect("insert backend fact");
        store
            .insert_or_update_memory(&frontend_mem)
            .expect("insert frontend mem");
        store
            .upsert_failure_pattern(&backend_failure)
            .expect("insert backend failure");

        let core_rules = vec![
            CoreRule::new(
                "rust-lifetime-rule",
                "Explicit Error Types",
                "All public crate functions must return Result<T, StrataError>",
            )
            .with_path_glob("crates/**/*.rs"),
            CoreRule::new(
                "tsx-a11y-rule",
                "Accessible Buttons",
                "All icon buttons in TSX must include aria-label",
            )
            .with_path_glob("web/src/**/*.tsx"),
        ];

        let compiler = MultiHostCompiler::new(Arc::clone(&store));
        let config = ContextBudgetConfig::new(2048, 20);

        // 1. Editing a Frontend file: web/src/components/App.tsx
        let frontend_ctx = compiler
            .compile_context_for_file(
                &config,
                Some("web/src/components/App.tsx"),
                &core_rules,
            )
            .expect("compile for frontend");
        assert!(frontend_ctx.contains("Always run tests before committing"));
        assert!(frontend_ctx.contains("Use React functional components with hooks"));
        assert!(frontend_ctx.contains("Accessible Buttons"));
        assert!(!frontend_ctx.contains("Use rusqlite with WAL mode"));
        assert!(!frontend_ctx.contains("UnwrapPanicInProd"));
        assert!(!frontend_ctx.contains("Explicit Error Types"));

        // 2. Editing a Backend Rust file: crates/strata-memory/src/compiler.rs
        let backend_ctx = compiler
            .compile_context_for_file(
                &config,
                Some("crates/strata-memory/src/compiler.rs"),
                &core_rules,
            )
            .expect("compile for backend");
        assert!(backend_ctx.contains("Always run tests before committing"));
        assert!(backend_ctx.contains("Use rusqlite with WAL mode"));
        assert!(backend_ctx.contains("UnwrapPanicInProd"));
        assert!(backend_ctx.contains("Explicit Error Types"));
        assert!(!backend_ctx.contains("Use React functional components with hooks"));
        assert!(!backend_ctx.contains("Accessible Buttons"));
    }

    #[test]
    fn test_compile_scoped_rules_generates_cursor_and_claude_files() {
        let store = Arc::new(SqliteStore::open_in_memory().expect("open store"));

        let backend_fact = SemanticFact::new(
            "Prefer zero-copy slices in parser hot paths",
            "rust_perf",
            Scope::Global,
        )
        .with_path_glob("crates/**/*.rs");
        let frontend_fact = SemanticFact::new(
            "Co-locate component styles with TSX files",
            "frontend_arch",
            Scope::Global,
        )
        .with_path_glob("src/frontend/**/*.tsx");

        store
            .insert_or_update_semantic_fact(&backend_fact)
            .expect("insert backend fact");
        store
            .insert_or_update_semantic_fact(&frontend_fact)
            .expect("insert frontend fact");

        let core_rules = vec![CoreRule::new(
            "frontend-strict-props",
            "Strict Props Interface",
            "Export explicit Props interface for every screen component",
        )
        .with_path_glob("src/frontend/**/*.tsx")];

        let compiler = MultiHostCompiler::new(Arc::clone(&store));
        let temp_workspace =
            std::env::temp_dir().join(format!("strata_scoped_rules_test_{}", Uuid::new_v4()));
        fs::create_dir_all(&temp_workspace).expect("create temp workspace");

        let report = compiler
            .compile_scoped_rules(
                &temp_workspace,
                &["cursor", "claude"],
                &core_rules,
                2048,
            )
            .expect("compile scoped rules");

        assert_eq!(report.target_hosts.len(), 4);

        let cursor_fe = temp_workspace
            .join(".cursor")
            .join("rules")
            .join("strata-scoped-src-frontend-tsx.mdc");
        let claude_fe = temp_workspace
            .join(".claude")
            .join("rules")
            .join("strata-scoped-src-frontend-tsx.md");
        let cursor_be = temp_workspace
            .join(".cursor")
            .join("rules")
            .join("strata-scoped-crates-rs.mdc");
        let claude_be = temp_workspace
            .join(".claude")
            .join("rules")
            .join("strata-scoped-crates-rs.md");

        assert!(cursor_fe.exists());
        assert!(claude_fe.exists());
        assert!(cursor_be.exists());
        assert!(claude_be.exists());

        let cursor_fe_content = fs::read_to_string(&cursor_fe).expect("read cursor fe");
        assert!(cursor_fe_content.contains("globs: src/frontend/**/*.tsx"));
        assert!(cursor_fe_content.contains("alwaysApply: false"));
        assert!(cursor_fe_content.contains("Co-locate component styles with TSX files"));
        assert!(cursor_fe_content.contains("Strict Props Interface"));
        assert!(!cursor_fe_content.contains("Prefer zero-copy slices"));

        let claude_fe_content = fs::read_to_string(&claude_fe).expect("read claude fe");
        assert!(claude_fe_content.contains("paths: src/frontend/**/*.tsx"));
        assert!(claude_fe_content.contains("Co-locate component styles with TSX files"));
        assert!(!claude_fe_content.contains("Prefer zero-copy slices"));

        let _ = fs::remove_dir_all(&temp_workspace);
    }
}

