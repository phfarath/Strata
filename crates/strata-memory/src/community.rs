use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use uuid::Uuid;

use crate::ast::LanguageKind;
use crate::call_graph::{CallEdge, CallGraph, CallGraphAnalyzer, CallType};
use strata_core::errors::StrataError;

/// Type of member node inside an architectural cluster.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemberType {
    Module,
    File,
    Function,
    Struct,
    Interface,
    MemoryNode,
}

impl std::fmt::Display for MemberType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MemberType::Module => write!(f, "module"),
            MemberType::File => write!(f, "file"),
            MemberType::Function => write!(f, "function"),
            MemberType::Struct => write!(f, "struct"),
            MemberType::Interface => write!(f, "interface"),
            MemberType::MemoryNode => write!(f, "memory_node"),
        }
    }
}

/// A constituent member (file, symbol, or module) belonging to an architectural cluster.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClusterMember {
    pub id: String,
    pub name: String,
    pub file_path: String,
    pub member_type: MemberType,
    pub internal_degree: usize,
    pub external_degree: usize,
}

/// Inter-cluster dependency edge representing calls or imports across clusters.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClusterDependency {
    pub target_cluster_id: String,
    pub target_cluster_name: String,
    pub edge_count: usize,
    pub sample_connections: Vec<String>,
}

/// A logical architectural community cluster grouping related modules and functions.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArchitectureCluster {
    pub id: String,
    pub name: String,
    pub description: String,
    pub members: Vec<ClusterMember>,
    pub cohesion: f64,
    pub coupling: f64,
    pub dependencies: Vec<ClusterDependency>,
    pub summary: Option<String>,
}

/// Comprehensive high-level summary of the codebase architecture graph.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArchitectureGraphSummary {
    pub id: Uuid,
    pub workspace_id: String,
    pub total_nodes: usize,
    pub total_edges: usize,
    pub modularity: f64,
    pub clusters: Vec<ArchitectureCluster>,
    pub cross_cluster_edges_count: usize,
    pub formatted_summary: String,
    pub created_at: DateTime<Utc>,
}

/// Configuration options for the Community Detection and Clustering algorithm.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClusteringConfig {
    /// Maximum number of LPA iterations before stopping (default: 25)
    pub max_iterations: usize,
    /// Minimum members required to form an independent cluster (default: 1)
    pub min_cluster_size: usize,
    /// Weight multiplier for direct function/method calls vs imports (default: 1.5)
    pub call_weight: f64,
    /// Weight multiplier for module imports (default: 1.0)
    pub import_weight: f64,
}

impl Default for ClusteringConfig {
    fn default() -> Self {
        Self {
            max_iterations: 25,
            min_cluster_size: 1,
            call_weight: 1.5,
            import_weight: 1.0,
        }
    }
}

/// Deterministic Graph Community Detection and Architecture Clustering Engine.
pub struct CommunityDetector {
    config: ClusteringConfig,
}

impl Default for CommunityDetector {
    fn default() -> Self {
        Self::new(ClusteringConfig::default())
    }
}

impl CommunityDetector {
    pub fn new(config: ClusteringConfig) -> Self {
        Self { config }
    }

    /// Clusters an existing `CallGraph` into architectural communities.
    pub fn detect_communities(
        &self,
        call_graph: &CallGraph,
        workspace_id: &str,
    ) -> ArchitectureGraphSummary {
        self.detect_from_edges(&call_graph.edges, workspace_id)
    }

    /// Clusters a slice of `CallEdge` records into architectural communities.
    pub fn detect_from_edges(
        &self,
        edges: &[CallEdge],
        workspace_id: &str,
    ) -> ArchitectureGraphSummary {
        let mut node_set: BTreeSet<String> = BTreeSet::new();
        let mut node_to_file: HashMap<String, String> = HashMap::new();
        let mut node_to_type: HashMap<String, MemberType> = HashMap::new();

        // 1. Collect all nodes and their attributes
        for edge in edges {
            let caller_node =
                if edge.caller_symbol == "<top-level>" || edge.caller_symbol.is_empty() {
                    edge.caller_file.clone()
                } else {
                    format!("{}::{}", edge.caller_file, edge.caller_symbol)
                };

            let callee_node = if let Some(ref hint) = edge.callee_file_hint {
                format!("{}::{}", hint, edge.callee_symbol)
            } else {
                edge.callee_symbol.clone()
            };

            node_set.insert(caller_node.clone());
            node_set.insert(callee_node.clone());

            node_to_file.insert(caller_node.clone(), edge.caller_file.clone());
            if edge.caller_symbol == "<top-level>" || edge.caller_symbol.is_empty() {
                node_to_type.insert(caller_node, MemberType::File);
            } else {
                node_to_type.insert(caller_node, MemberType::Function);
            }

            if let Some(ref hint) = edge.callee_file_hint {
                node_to_file.insert(callee_node.clone(), hint.clone());
            } else {
                node_to_file
                    .entry(callee_node.clone())
                    .or_insert_with(|| edge.caller_file.clone());
            }

            node_to_type
                .entry(callee_node)
                .or_insert(match edge.call_type {
                    CallType::Import => MemberType::Module,
                    CallType::ConstructorCall => MemberType::Struct,
                    _ => MemberType::Function,
                });
        }

        let nodes: Vec<String> = node_set.into_iter().collect();
        let n = nodes.len();

        if n == 0 {
            return ArchitectureGraphSummary {
                id: Uuid::new_v4(),
                workspace_id: workspace_id.to_string(),
                total_nodes: 0,
                total_edges: 0,
                modularity: 0.0,
                clusters: Vec::new(),
                cross_cluster_edges_count: 0,
                formatted_summary:
                    "No code symbols or call edges detected for architectural clustering."
                        .to_string(),
                created_at: Utc::now(),
            };
        }

        let mut node_idx_map: HashMap<&str, usize> = HashMap::new();
        for (i, node) in nodes.iter().enumerate() {
            node_idx_map.insert(node.as_str(), i);
        }

        // 2. Build weighted undirected adjacency matrix / map for clustering
        let mut adj: Vec<BTreeMap<usize, f64>> = vec![BTreeMap::new(); n];
        let mut total_weight = 0.0;

        for edge in edges {
            let caller_node =
                if edge.caller_symbol == "<top-level>" || edge.caller_symbol.is_empty() {
                    &edge.caller_file
                } else {
                    &format!("{}::{}", edge.caller_file, edge.caller_symbol)
                };

            let callee_node = if let Some(ref hint) = edge.callee_file_hint {
                format!("{}::{}", hint, edge.callee_symbol)
            } else {
                edge.callee_symbol.clone()
            };

            if let (Some(&u), Some(&v)) = (
                node_idx_map.get(caller_node.as_str()),
                node_idx_map.get(callee_node.as_str()),
            ) {
                let weight = match edge.call_type {
                    CallType::Import => self.config.import_weight,
                    _ => self.config.call_weight,
                };

                *adj[u].entry(v).or_insert(0.0) += weight;
                *adj[v].entry(u).or_insert(0.0) += weight;
                total_weight += weight;
            }
        }

        // Add small intra-file structural affinity if nodes are in the same file
        for i in 0..n {
            for j in (i + 1)..n {
                let file_i = node_to_file.get(&nodes[i]);
                let file_j = node_to_file.get(&nodes[j]);
                if let (Some(fi), Some(fj)) = (file_i, file_j) {
                    if fi == fj && !fi.is_empty() {
                        let affinity = 0.5;
                        *adj[i].entry(j).or_insert(0.0) += affinity;
                        *adj[j].entry(i).or_insert(0.0) += affinity;
                        total_weight += affinity;
                    }
                }
            }
        }

        // 3. Deterministic Label Propagation Algorithm (LPA)
        // Initialize each node with initial label based on file directory path or its own index
        let mut labels: Vec<usize> = Vec::with_capacity(n);
        let mut initial_label_map: HashMap<String, usize> = HashMap::new();
        let mut next_label_id = 0;

        for node in &nodes {
            let file = node_to_file.get(node).cloned().unwrap_or_default();
            let parent_dir = Path::new(&file)
                .parent()
                .and_then(|p| p.to_str())
                .unwrap_or("")
                .to_string();

            let label_key = if !parent_dir.is_empty() {
                parent_dir
            } else {
                file
            };

            let label = *initial_label_map.entry(label_key).or_insert_with(|| {
                let id = next_label_id;
                next_label_id += 1;
                id
            });
            labels.push(label);
        }

        // Iterative label propagation
        for _iter in 0..self.config.max_iterations {
            let mut changed = false;

            // Visit nodes in deterministic order (0..n)
            for u in 0..n {
                if adj[u].is_empty() {
                    continue;
                }

                // Sum weights per neighbor label
                let mut label_weights: BTreeMap<usize, f64> = BTreeMap::new();
                for (&v, &w) in &adj[u] {
                    let v_label = labels[v];
                    *label_weights.entry(v_label).or_insert(0.0) += w;
                }

                // Find maximal label with deterministic tie-breaking (smallest label id)
                let mut best_label = labels[u];
                let mut max_weight = -1.0;

                for (&lbl, &w) in &label_weights {
                    if w > max_weight + 1e-9 {
                        max_weight = w;
                        best_label = lbl;
                    }
                }

                if best_label != labels[u] {
                    labels[u] = best_label;
                    changed = true;
                }
            }

            if !changed {
                break;
            }
        }

        // 4. Group nodes into raw communities
        let mut raw_communities: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
        for (node_idx, &label) in labels.iter().enumerate() {
            raw_communities.entry(label).or_default().push(node_idx);
        }

        // Filter or merge tiny communities if needed
        let mut cluster_list: Vec<Vec<usize>> = Vec::new();
        for (_lbl, members) in raw_communities {
            if members.len() >= self.config.min_cluster_size {
                cluster_list.push(members);
            }
        }

        // If all filtered out, keep the largest
        if cluster_list.is_empty() && !nodes.is_empty() {
            cluster_list.push((0..n).collect());
        }

        // Sort clusters by size descending for deterministic presentation
        cluster_list.sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| a[0].cmp(&b[0])));

        // Map each node to its final cluster index
        let mut node_to_cluster_idx: Vec<usize> = vec![0; n];
        for (c_idx, c_members) in cluster_list.iter().enumerate() {
            for &m in c_members {
                node_to_cluster_idx[m] = c_idx;
            }
        }

        // 5. Calculate Modularity Q (Newman-Girvan Modularity)
        let modularity = if total_weight > 0.0 {
            let two_m = 2.0 * total_weight;
            let mut q_sum = 0.0;
            for (c_idx, c_members) in cluster_list.iter().enumerate() {
                let mut internal_weight = 0.0;
                let mut degree_sum = 0.0;

                for &u in c_members {
                    let mut deg_u = 0.0;
                    for (&v, &w) in &adj[u] {
                        deg_u += w;
                        if node_to_cluster_idx[v] == c_idx {
                            internal_weight += w;
                        }
                    }
                    degree_sum += deg_u;
                }

                internal_weight /= 2.0; // each internal edge counted twice in adj
                q_sum += (internal_weight / total_weight) - (degree_sum / two_m).powi(2);
            }
            q_sum.clamp(-1.0, 1.0)
        } else {
            0.0
        };

        // 6. Build ArchitectureCluster objects
        let mut clusters = Vec::new();
        let mut total_cross_edges = 0;

        for (c_idx, c_members) in cluster_list.iter().enumerate() {
            let mut member_objects = Vec::new();
            let mut internal_edges_count = 0.0;
            let mut external_edges_count = 0.0;
            let mut dep_map: BTreeMap<usize, (usize, Vec<String>)> = BTreeMap::new();

            for &u in c_members {
                let node_id = &nodes[u];
                let file_path = node_to_file.get(node_id).cloned().unwrap_or_default();
                let m_type = node_to_type
                    .get(node_id)
                    .cloned()
                    .unwrap_or(MemberType::Function);
                let simple_name = node_id.split("::").last().unwrap_or(node_id).to_string();

                let mut int_deg = 0;
                let mut ext_deg = 0;

                for (&v, &w) in &adj[u] {
                    let target_cluster = node_to_cluster_idx[v];
                    if target_cluster == c_idx {
                        int_deg += 1;
                        internal_edges_count += w;
                    } else {
                        ext_deg += 1;
                        external_edges_count += w;
                        let entry = dep_map
                            .entry(target_cluster)
                            .or_insert_with(|| (0, Vec::new()));
                        entry.0 += 1;
                        if entry.1.len() < 3 {
                            entry.1.push(format!("{} -> {}", node_id, nodes[v]));
                        }
                    }
                }

                member_objects.push(ClusterMember {
                    id: node_id.clone(),
                    name: simple_name,
                    file_path,
                    member_type: m_type,
                    internal_degree: int_deg,
                    external_degree: ext_deg,
                });
            }

            internal_edges_count /= 2.0;
            total_cross_edges += external_edges_count as usize;

            let cohesion = if (internal_edges_count + external_edges_count) > 0.0 {
                internal_edges_count / (internal_edges_count + external_edges_count)
            } else {
                1.0
            };

            let coupling = if total_weight > 0.0 {
                external_edges_count / (total_weight / 2.0)
            } else {
                0.0
            };

            // Infer meaningful domain name and description for this cluster
            let (cluster_name, cluster_desc) = self.infer_cluster_identity(&member_objects, c_idx);
            let cluster_id = format!("cluster-{}-{}", c_idx + 1, sanitize_slug(&cluster_name));

            clusters.push(ArchitectureCluster {
                id: cluster_id,
                name: cluster_name,
                description: cluster_desc,
                members: member_objects,
                cohesion: (cohesion * 100.0).round() / 100.0,
                coupling: (coupling * 100.0).round() / 100.0,
                dependencies: Vec::new(), // Will populate after names are all resolved
                summary: None,
            });
        }

        // Populate dependencies with resolved cluster target names
        for (c_idx, c_members) in cluster_list.iter().enumerate() {
            let mut dep_map: BTreeMap<usize, (usize, Vec<String>)> = BTreeMap::new();
            for &u in c_members {
                for (&v, _) in &adj[u] {
                    let target_cluster = node_to_cluster_idx[v];
                    if target_cluster != c_idx {
                        let entry = dep_map
                            .entry(target_cluster)
                            .or_insert_with(|| (0, Vec::new()));
                        entry.0 += 1;
                        if entry.1.len() < 3 {
                            entry.1.push(format!("{} -> {}", nodes[u], nodes[v]));
                        }
                    }
                }
            }

            let mut deps = Vec::new();
            for (target_c, (cnt, samples)) in dep_map {
                if target_c < clusters.len() {
                    deps.push(ClusterDependency {
                        target_cluster_id: clusters[target_c].id.clone(),
                        target_cluster_name: clusters[target_c].name.clone(),
                        edge_count: cnt,
                        sample_connections: samples,
                    });
                }
            }
            clusters[c_idx].dependencies = deps;
        }

        // 7. Render high-level Markdown summary
        let formatted_summary = self.render_markdown_summary(
            workspace_id,
            nodes.len(),
            edges.len(),
            modularity,
            &clusters,
            total_cross_edges / 2,
        );

        ArchitectureGraphSummary {
            id: Uuid::new_v4(),
            workspace_id: workspace_id.to_string(),
            total_nodes: nodes.len(),
            total_edges: edges.len(),
            modularity: (modularity * 1000.0).round() / 1000.0,
            clusters,
            cross_cluster_edges_count: total_cross_edges / 2,
            formatted_summary,
            created_at: Utc::now(),
        }
    }

    /// Scans a workspace directory, analyzes all source code files, and builds an architectural clustering map.
    pub fn detect_from_directory(
        &self,
        root_dir: &Path,
        workspace_id: &str,
    ) -> Result<ArchitectureGraphSummary, StrataError> {
        if !root_dir.exists() {
            return Err(StrataError::Validation(format!(
                "Directory does not exist: {}",
                root_dir.display()
            )));
        }

        let mut files = Vec::new();
        collect_source_files(root_dir, &mut files);

        let analyzer = CallGraphAnalyzer::new();
        let mut all_edges = Vec::new();

        for p in files {
            let p_str = p.to_string_lossy();
            let lang = LanguageKind::from_file_path(&p_str);
            if lang != LanguageKind::Unknown {
                if let Ok(content) = std::fs::read_to_string(&p) {
                    if let Ok(edges) = analyzer.analyze_source(&content, lang, &p_str) {
                        all_edges.extend(edges);
                    }
                }
            }
        }

        Ok(self.detect_from_edges(&all_edges, workspace_id))
    }

    // ==========================================
    // Heuristic Cluster Naming & Domain Detection
    // ==========================================

    fn infer_cluster_identity(&self, members: &[ClusterMember], index: usize) -> (String, String) {
        let mut dir_counts: HashMap<String, usize> = HashMap::new();
        let mut keywords: HashSet<String> = HashSet::new();

        for m in members {
            let path = Path::new(&m.file_path);
            if let Some(parent) = path.parent() {
                let dir_name = parent.file_name().and_then(|f| f.to_str()).unwrap_or("");
                if !dir_name.is_empty() && dir_name != "src" && dir_name != "crates" {
                    *dir_counts.entry(dir_name.to_lowercase()).or_insert(0) += 1;
                }
            }

            let text_to_check = format!("{} {}", m.name.to_lowercase(), m.file_path.to_lowercase());
            for kw in &[
                "auth",
                "login",
                "jwt",
                "token",
                "user",
                "security",
                "permission",
                "password",
                "db",
                "store",
                "sqlite",
                "postgres",
                "sql",
                "migration",
                "repository",
                "table",
                "sync",
                "cdc",
                "delta",
                "outbox",
                "replication",
                "stream",
                "event",
                "api",
                "route",
                "handler",
                "server",
                "http",
                "axum",
                "endpoint",
                "socket",
                "cli",
                "command",
                "args",
                "terminal",
                "prompt",
                "interactive",
                "decay",
                "ebbinghaus",
                "prune",
                "memory",
                "retrieval",
                "search",
                "fts",
                "ast",
                "anchor",
                "tree_sitter",
                "parser",
                "diff",
                "merkle",
                "eval",
                "bench",
                "scenario",
                "test",
                "verification",
                "reasoning",
                "dag",
                "plan",
                "train",
                "lora",
                "distill",
                "workspace",
                "monorepo",
                "boundary",
                "package",
                "crate",
            ] {
                if text_to_check.contains(kw) {
                    keywords.insert(kw.to_string());
                }
            }
        }

        // Check for dominant architectural patterns
        if keywords.contains("auth")
            || keywords.contains("login")
            || keywords.contains("jwt")
            || keywords.contains("security")
        {
            return (
                "Authentication & Security".to_string(),
                "Identity verification, credential management, tokens and access policies."
                    .to_string(),
            );
        }
        if keywords.contains("store")
            || keywords.contains("sqlite")
            || keywords.contains("postgres")
            || keywords.contains("db")
        {
            return (
                "Database & Persistence".to_string(),
                "Relational storage, entity schemas, migrations and database operations."
                    .to_string(),
            );
        }
        if keywords.contains("sync")
            || keywords.contains("cdc")
            || keywords.contains("outbox")
            || keywords.contains("delta")
        {
            return (
                "Synchronization & CDC Engine".to_string(),
                "Offline-first change data capture, conflict resolution and sync replication."
                    .to_string(),
            );
        }
        if keywords.contains("api")
            || keywords.contains("server")
            || keywords.contains("axum")
            || keywords.contains("http")
        {
            return (
                "HTTP API & Server Routing".to_string(),
                "Network endpoints, protocol handlers and server request processing.".to_string(),
            );
        }
        if keywords.contains("cli") || keywords.contains("command") || keywords.contains("terminal")
        {
            return (
                "CLI & Command Gateway".to_string(),
                "User command-line interfaces, options parsing and execution commands.".to_string(),
            );
        }
        if keywords.contains("decay")
            || keywords.contains("retrieval")
            || keywords.contains("search")
            || keywords.contains("fts")
        {
            return (
                "Cognitive Memory & Retrieval".to_string(),
                "Mathematical decay, full-text lexical search and semantic memory recall."
                    .to_string(),
            );
        }
        if keywords.contains("ast") || keywords.contains("anchor") || keywords.contains("parser") {
            return (
                "AST Code Anchoring & Syntax".to_string(),
                "Syntax tree parsing, symbol extraction and code anchoring across commits."
                    .to_string(),
            );
        }
        if keywords.contains("workspace")
            || keywords.contains("monorepo")
            || keywords.contains("package")
        {
            return (
                "Workspace & Monorepo Boundaries".to_string(),
                "Package isolation, dependency analysis and monorepo boundaries.".to_string(),
            );
        }
        if keywords.contains("dag") || keywords.contains("plan") || keywords.contains("reasoning") {
            return (
                "Planning & Reasoning DAG".to_string(),
                "Hierarchical goal decomposition, causal reasoning and task execution waves."
                    .to_string(),
            );
        }
        if keywords.contains("eval") || keywords.contains("bench") || keywords.contains("scenario")
        {
            return (
                "Evaluation & Benchmarks".to_string(),
                "Automated verification scenarios and performance evaluation benchmarks."
                    .to_string(),
            );
        }

        // Fallback to dominant directory
        if let Some((top_dir, _)) = dir_counts.into_iter().max_by_key(|(_, c)| *c) {
            let capitalized = capitalize_first(&top_dir);
            return (
                format!("{capitalized} Subsystem"),
                format!("Core logic and operations related to `{top_dir}` components."),
            );
        }

        (
            format!("Architecture Cluster {}", index + 1),
            "Cohesive group of interrelated functions, files and modules.".to_string(),
        )
    }

    fn render_markdown_summary(
        &self,
        workspace_id: &str,
        total_nodes: usize,
        total_edges: usize,
        modularity: f64,
        clusters: &[ArchitectureCluster],
        cross_edges: usize,
    ) -> String {
        let mut lines = Vec::new();
        lines.push(format!(
            "### 🏛️ High-Level Architecture Map (`{workspace_id}`)"
        ));
        lines.push(format!(
            "- **Total Nodes**: {} symbols/files | **Call/Import Edges**: {} | **Modularity (Q)**: {:.3}",
            total_nodes, total_edges, modularity
        ));
        lines.push(format!(
            "- **Extracted Communities**: {} clusters | **Cross-Cluster Boundaries**: {} edges\n",
            clusters.len(),
            cross_edges
        ));

        for (i, c) in clusters.iter().enumerate() {
            lines.push(format!("#### {}. 📦 **{}** (`{}`)", i + 1, c.name, c.id));
            lines.push(format!("> _{}_", c.description));
            lines.push(format!(
                "- **Metrics**: Cohesion: `{:.2}` | Coupling: `{:.2}` | Members: `{}` items",
                c.cohesion,
                c.coupling,
                c.members.len()
            ));

            // Show top members
            let preview_members: Vec<String> = c
                .members
                .iter()
                .take(5)
                .map(|m| format!("`{}` ({})", m.name, m.member_type))
                .collect();
            let more_suffix = if c.members.len() > 5 {
                format!(" _(+{} more)_", c.members.len() - 5)
            } else {
                "".to_string()
            };
            lines.push(format!(
                "- **Key Members**: {}{}",
                preview_members.join(", "),
                more_suffix
            ));

            // Show inter-cluster dependencies
            if !c.dependencies.is_empty() {
                let dep_strs: Vec<String> = c
                    .dependencies
                    .iter()
                    .map(|d| format!("{} ({} edges)", d.target_cluster_name, d.edge_count))
                    .collect();
                lines.push(format!("- **Interacts with**: {}", dep_strs.join(", ")));
            }
            lines.push("".to_string());
        }

        lines.join("\n")
    }
}

fn collect_source_files(dir: &Path, files: &mut Vec<PathBuf>) {
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.filter_map(|e| e.ok()) {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with('.')
                || name == "target"
                || name == "node_modules"
                || name == "dist"
                || name == "build"
            {
                continue;
            }
            if path.is_dir() {
                collect_source_files(&path, files);
            } else if path.is_file() {
                let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
                if matches!(ext, "rs" | "ts" | "tsx" | "js" | "jsx" | "py") {
                    files.push(path);
                }
            }
        }
    }
}

fn capitalize_first(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        None => String::new(),
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
    }
}

fn sanitize_slug(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

// ==========================================
// TDD Tests for Community Detection
// ==========================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::call_graph::{CallEdge, CallType};

    #[test]
    fn test_community_detection_and_metrics_calculation() {
        // Build synthetic call edges representing 2 distinct clusters: Auth and Database
        let mut edges = Vec::new();

        // Auth cluster
        edges.push(CallEdge::new(
            "src/auth/login.rs",
            "handle_login",
            "verify_password",
            10,
            CallType::FunctionCall,
        ));
        edges.push(CallEdge::new(
            "src/auth/login.rs",
            "handle_login",
            "generate_jwt",
            15,
            CallType::FunctionCall,
        ));
        edges.push(CallEdge::new(
            "src/auth/token.rs",
            "generate_jwt",
            "sign_claims",
            20,
            CallType::FunctionCall,
        ));

        // DB cluster
        edges.push(CallEdge::new(
            "src/db/store.rs",
            "query_record",
            "execute_sql",
            50,
            CallType::FunctionCall,
        ));
        edges.push(CallEdge::new(
            "src/db/store.rs",
            "insert_record",
            "execute_sql",
            55,
            CallType::FunctionCall,
        ));
        edges.push(CallEdge::new(
            "src/db/pool.rs",
            "connect_db",
            "create_pool",
            30,
            CallType::FunctionCall,
        ));
        edges.push(CallEdge::new(
            "src/db/store.rs",
            "query_record",
            "connect_db",
            60,
            CallType::FunctionCall,
        ));

        // Inter-cluster bridge edge: Login calls query_record to find user
        edges.push(CallEdge::new(
            "src/auth/login.rs",
            "handle_login",
            "query_record",
            12,
            CallType::FunctionCall,
        ));

        let detector = CommunityDetector::default();
        let summary = detector.detect_from_edges(&edges, "test-workspace");

        assert!(summary.total_nodes >= 6, "Expected at least 6 unique nodes");
        assert_eq!(summary.total_edges, 8);
        assert!(!summary.clusters.is_empty(), "Should extract communities");

        // Verify clusters have cohesion and coupling metrics calculated
        for cluster in &summary.clusters {
            assert!(cluster.cohesion >= 0.0 && cluster.cohesion <= 1.0);
            assert!(cluster.coupling >= 0.0 && cluster.coupling <= 1.0);
            assert!(!cluster.members.is_empty());
        }

        // Verify markdown summary formatting
        assert!(summary
            .formatted_summary
            .contains("High-Level Architecture Map"));
        assert!(summary.formatted_summary.contains("Modularity"));
    }

    #[test]
    fn test_multi_language_source_clustering() {
        let rust_auth = r#"
pub fn authenticate(token: &str) -> bool {
    validate_jwt(token)
}
fn validate_jwt(t: &str) -> bool {
    !t.is_empty()
}
"#;
        let ts_api = r#"
import { authenticate } from './auth';
export function handleRequest(req: any) {
    if (authenticate(req.token)) {
        return sendJson(200);
    }
    return sendJson(401);
}
function sendJson(code: number) { return code; }
"#;

        let analyzer = CallGraphAnalyzer::new();
        let mut all_edges = Vec::new();
        all_edges.extend(
            analyzer
                .analyze_source(rust_auth, LanguageKind::Rust, "src/auth.rs")
                .unwrap(),
        );
        all_edges.extend(
            analyzer
                .analyze_source(ts_api, LanguageKind::TypeScript, "src/api.ts")
                .unwrap(),
        );

        let detector = CommunityDetector::default();
        let summary = detector.detect_from_edges(&all_edges, "polyglot-workspace");

        assert!(summary.total_nodes >= 3);
        assert!(!summary.clusters.is_empty());
    }
}
