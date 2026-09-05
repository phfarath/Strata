use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};
use tracing::info;

use super::graph::CausalGraph;
use super::types::{CausalEdge, CausalEdgeKind, CausalNode, CausalNodeKind};

/// Indexer that scans Rust workspaces and codebases, extracting architectural topologies and causal coupling.
pub struct CodebaseCausalIndexer {
    ignored_dirs: Vec<String>,
}

impl Default for CodebaseCausalIndexer {
    fn default() -> Self {
        Self::new()
    }
}

impl CodebaseCausalIndexer {
    pub fn new() -> Self {
        Self {
            ignored_dirs: vec![
                "target".to_string(),
                ".git".to_string(),
                "node_modules".to_string(),
                ".vite".to_string(),
                "dist".to_string(),
            ],
        }
    }

    /// Index workspace starting from `root_dir` into the given `CausalGraph`.
    /// Returns count of nodes indexed.
    pub fn index_workspace(&self, root_dir: &Path, graph: &mut CausalGraph) -> Result<usize> {
        info!(
            "Indexing codebase causal topology from: {}",
            root_dir.display()
        );

        let initial_node_count = graph.node_count();

        // 1. Seed Core Strata Domain Invariants & Infrastructure Nodes
        self.seed_core_architecture_topology(graph);

        // 2. Discover and parse workspace crates
        let mut rust_files = Vec::new();
        self.collect_rust_files(root_dir, &mut rust_files)?;

        for file_path in &rust_files {
            self.index_rust_file(root_dir, file_path, graph)?;
        }

        let new_nodes = graph.node_count() - initial_node_count;
        info!(
            "Indexed {} files, total causal graph nodes: {}, edges: {}",
            rust_files.len(),
            graph.node_count(),
            graph.edge_count()
        );

        Ok(new_nodes)
    }

    /// Recursively collect all `.rs` files outside ignored directories.
    fn collect_rust_files(&self, dir: &Path, results: &mut Vec<PathBuf>) -> Result<()> {
        if !dir.exists() || !dir.is_dir() {
            return Ok(());
        }

        for entry in fs::read_dir(dir).with_context(|| format!("Reading dir {:?}", dir))? {
            let entry = entry?;
            let path = entry.path();
            let file_name = entry.file_name().to_string_lossy().to_string();

            if self.ignored_dirs.contains(&file_name) {
                continue;
            }

            if path.is_dir() {
                self.collect_rust_files(&path, results)?;
            } else if path.extension().and_then(|s| s.to_str()) == Some("rs") {
                results.push(path);
            }
        }

        Ok(())
    }

    /// Parse a single Rust file and extract its module declaration, struct definitions, and imports.
    fn index_rust_file(
        &self,
        root_dir: &Path,
        file_path: &Path,
        graph: &mut CausalGraph,
    ) -> Result<()> {
        let relative_path = file_path
            .strip_prefix(root_dir)
            .unwrap_or(file_path)
            .to_string_lossy()
            .replace('\\', "/");

        let _file_stem = file_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown");

        let file_node_id = format!("file:{relative_path}");
        let file_node = CausalNode::new(
            file_node_id.clone(),
            format!("File: {relative_path}"),
            CausalNodeKind::File,
        )
        .with_path(relative_path.clone());

        graph.add_node(file_node);

        let content = match fs::read_to_string(file_path) {
            Ok(c) => c,
            Err(_) => return Ok(()),
        };

        // Extract structs, enums, traits, functions and link to file
        for line in content.lines() {
            let trimmed = line.trim();

            if trimmed.starts_with("pub struct ") || trimmed.starts_with("struct ") {
                if let Some(name) = self.extract_item_name(trimmed, "struct") {
                    let struct_id = format!("struct:{relative_path}::{name}");
                    let node = CausalNode::new(
                        struct_id.clone(),
                        format!("struct {name}"),
                        CausalNodeKind::Struct,
                    )
                    .with_path(relative_path.clone());
                    graph.add_node(node);
                    let _ = graph.add_edge(
                        &file_node_id,
                        &struct_id,
                        CausalEdge::new(CausalEdgeKind::Extends, 1.0, true),
                    );
                }
            } else if trimmed.starts_with("pub enum ") || trimmed.starts_with("enum ") {
                if let Some(name) = self.extract_item_name(trimmed, "enum") {
                    let enum_id = format!("enum:{relative_path}::{name}");
                    let node = CausalNode::new(
                        enum_id.clone(),
                        format!("enum {name}"),
                        CausalNodeKind::Enum,
                    )
                    .with_path(relative_path.clone());
                    graph.add_node(node);
                    let _ = graph.add_edge(
                        &file_node_id,
                        &enum_id,
                        CausalEdge::new(CausalEdgeKind::Extends, 1.0, true),
                    );
                }
            } else if trimmed.starts_with("pub trait ") || trimmed.starts_with("trait ") {
                if let Some(name) = self.extract_item_name(trimmed, "trait") {
                    let trait_id = format!("trait:{relative_path}::{name}");
                    let node = CausalNode::new(
                        trait_id.clone(),
                        format!("trait {name}"),
                        CausalNodeKind::Trait,
                    )
                    .with_path(relative_path.clone());
                    graph.add_node(node);
                    let _ = graph.add_edge(
                        &file_node_id,
                        &trait_id,
                        CausalEdge::new(CausalEdgeKind::Extends, 1.0, true),
                    );
                }
            } else if trimmed.starts_with("use strata_") || trimmed.starts_with("use crate::") {
                // Dependency import edge
                if let Some(target_crate) = self.extract_imported_crate(trimmed) {
                    let crate_node_id = format!("crate:{target_crate}");
                    let _ = graph.add_edge(&file_node_id, &crate_node_id, CausalEdge::imports(0.8));
                }
            }
        }

        Ok(())
    }

    fn extract_item_name(&self, line: &str, keyword: &str) -> Option<String> {
        let after = line.split(keyword).nth(1)?.trim();
        let name = after.split(&['<', '{', '(', ';', ' '][..]).next()?.trim();
        if !name.is_empty() {
            Some(name.to_string())
        } else {
            None
        }
    }

    fn extract_imported_crate(&self, line: &str) -> Option<String> {
        let after_use = line.strip_prefix("use ")?.trim();
        let first_seg = after_use.split("::").next()?.trim();
        if first_seg.starts_with("strata_") {
            Some(first_seg.to_string())
        } else {
            None
        }
    }

    /// Seed well-known Strata architecture components, data tables, and contract invariants.
    pub fn seed_core_architecture_topology(&self, graph: &mut CausalGraph) {
        // --- Crates ---
        let crates = [
            (
                "crate:strata_core",
                "Crate: strata-core",
                "Core data models, schemas, events and error traits",
            ),
            (
                "crate:strata_memory",
                "Crate: strata-memory",
                "SQLite WAL, ACT-R decay, JTMS, hybrid search",
            ),
            (
                "crate:strata_tools",
                "Crate: strata-tools",
                "Security gateway, builtin memory tools, rate limiting",
            ),
            (
                "crate:strata_reasoning",
                "Crate: strata-reasoning",
                "World model, LLM adapters, distillation prompts",
            ),
            (
                "crate:strata_server",
                "Crate: strata-server",
                "Axum HTTP API, multi-tenant auth, pgvector sync",
            ),
            (
                "crate:strata_cli",
                "Crate: strata-cli",
                "CLI binary, MCP server, observability TUI",
            ),
            (
                "crate:strata_evals",
                "Crate: strata-evals",
                "Deterministic cognitive evaluation scenarios",
            ),
        ];

        for (id, name, desc) in crates {
            graph.add_node(
                CausalNode::new(id, name, CausalNodeKind::Module)
                    .with_metadata(serde_json::json!({ "description": desc })),
            );
        }

        // --- Crate Dependencies (who depends on who) ---
        // strata-memory -> strata-core
        let _ = graph.add_edge(
            "crate:strata_memory",
            "crate:strata_core",
            CausalEdge::imports(1.0),
        );
        // strata-tools -> strata-core, strata-memory
        let _ = graph.add_edge(
            "crate:strata_tools",
            "crate:strata_core",
            CausalEdge::imports(1.0),
        );
        // strata-reasoning -> strata-core, strata-memory
        let _ = graph.add_edge(
            "crate:strata_reasoning",
            "crate:strata_core",
            CausalEdge::imports(1.0),
        );
        let _ = graph.add_edge(
            "crate:strata_reasoning",
            "crate:strata_memory",
            CausalEdge::imports(0.9),
        );
        // strata-server -> strata-core, strata-memory
        let _ = graph.add_edge(
            "crate:strata_server",
            "crate:strata_core",
            CausalEdge::imports(1.0),
        );
        let _ = graph.add_edge(
            "crate:strata_server",
            "crate:strata_memory",
            CausalEdge::imports(0.8),
        );
        // strata-cli -> all
        let _ = graph.add_edge(
            "crate:strata_cli",
            "crate:strata_core",
            CausalEdge::imports(1.0),
        );
        let _ = graph.add_edge(
            "crate:strata_cli",
            "crate:strata_memory",
            CausalEdge::imports(1.0),
        );
        let _ = graph.add_edge(
            "crate:strata_cli",
            "crate:strata_tools",
            CausalEdge::imports(0.9),
        );
        let _ = graph.add_edge(
            "crate:strata_cli",
            "crate:strata_reasoning",
            CausalEdge::imports(0.9),
        );
        // strata-evals -> all
        let _ = graph.add_edge(
            "crate:strata_evals",
            "crate:strata_core",
            CausalEdge::imports(1.0),
        );
        let _ = graph.add_edge(
            "crate:strata_evals",
            "crate:strata_memory",
            CausalEdge::imports(1.0),
        );
        let _ = graph.add_edge(
            "crate:strata_evals",
            "crate:strata_cli",
            CausalEdge::imports(0.9),
        );

        // --- Core Database Tables ---
        let tables = [
            (
                "table:events",
                "Database Table: events",
                "Canonical event log stream",
            ),
            (
                "table:memories",
                "Database Table: memories",
                "General persistent memories & vector embeddings",
            ),
            (
                "table:semantic_facts",
                "Database Table: semantic_facts",
                "Atomic facts under JTMS truth maintenance",
            ),
            (
                "table:failure_patterns",
                "Database Table: failure_patterns",
                "Captured anti-patterns and mitigations",
            ),
            (
                "table:sync_outbox",
                "Database Table: sync_outbox",
                "CDC offline-first synchronization outbox",
            ),
            (
                "table:users",
                "Database Table: users",
                "SaaS multi-tenant developer accounts",
            ),
            (
                "table:api_keys",
                "Database Table: api_keys",
                "Hashed machine API keys for agent access",
            ),
        ];

        for (id, name, desc) in tables {
            graph.add_node(
                CausalNode::new(id, name, CausalNodeKind::DatabaseTable)
                    .with_metadata(serde_json::json!({ "description": desc })),
            );
        }

        // Storage writes to tables
        let _ = graph.add_edge(
            "crate:strata_memory",
            "table:events",
            CausalEdge::writes_to(1.0),
        );
        let _ = graph.add_edge(
            "crate:strata_memory",
            "table:memories",
            CausalEdge::writes_to(1.0),
        );
        let _ = graph.add_edge(
            "crate:strata_memory",
            "table:semantic_facts",
            CausalEdge::writes_to(1.0),
        );
        let _ = graph.add_edge(
            "crate:strata_memory",
            "table:failure_patterns",
            CausalEdge::writes_to(1.0),
        );
        let _ = graph.add_edge(
            "crate:strata_memory",
            "table:sync_outbox",
            CausalEdge::writes_to(1.0),
        );
        let _ = graph.add_edge(
            "crate:strata_server",
            "table:users",
            CausalEdge::writes_to(1.0),
        );
        let _ = graph.add_edge(
            "crate:strata_server",
            "table:api_keys",
            CausalEdge::writes_to(1.0),
        );

        // --- Axum API Endpoints ---
        let endpoints = [
            (
                "endpoint:POST /api/v1/sync/push",
                "API: POST /api/v1/sync/push",
                "crates/strata-server/src/handlers.rs",
            ),
            (
                "endpoint:GET /api/v1/sync/pull",
                "API: GET /api/v1/sync/pull",
                "crates/strata-server/src/handlers.rs",
            ),
            (
                "endpoint:GET /api/v1/sync/ws",
                "API: GET /api/v1/sync/ws",
                "crates/strata-server/src/handlers.rs",
            ),
            (
                "endpoint:POST /api/v1/auth/signup",
                "API: POST /api/v1/auth/signup",
                "crates/strata-server/src/handlers.rs",
            ),
            (
                "endpoint:POST /api/v1/auth/login",
                "API: POST /api/v1/auth/login",
                "crates/strata-server/src/handlers.rs",
            ),
            (
                "endpoint:POST /api/v1/keys",
                "API: POST /api/v1/keys",
                "crates/strata-server/src/handlers.rs",
            ),
        ];

        for (id, name, path) in endpoints {
            graph.add_node(CausalNode::new(id, name, CausalNodeKind::Endpoint).with_path(path));
            let _ = graph.add_edge("crate:strata_server", id, CausalEdge::exposes_endpoint(1.0));
        }

        // --- Architectural Invariants & Anti-Pattern Contracts ---
        let invariants = [
            (
                "invariant:strict_security_headers",
                "Invariant: HSTS & CSP Security Headers",
                "All HTTP responses must enforce HSTS 2y, X-Frame-Options: DENY, and CSP allowing WebSocket wss:",
                "crate:strata_server",
            ),
            (
                "invariant:offline_first_cdc",
                "Invariant: Offline-First CDC Monotonic Sequence",
                "Deltas must have monotonic sequences and deterministic conflict resolution",
                "crate:strata_memory",
            ),
            (
                "invariant:pure_rust_tls",
                "Anti-Pattern: Pure Rust TLS over OpenSSL",
                "Must use rustls + ring + webpki-roots to prevent OpenSSL dynamic link failures on distroless containers",
                "crate:strata_server",
            ),
        ];

        for (id, name, desc, target_node) in invariants {
            graph.add_node(
                CausalNode::new(id, name, CausalNodeKind::ContractInvariant)
                    .with_metadata(serde_json::json!({ "description": desc })),
            );
            let _ = graph.add_edge(
                id,
                target_node,
                CausalEdge::enforces_contract(1.0).with_description(desc),
            );
        }
    }
}
