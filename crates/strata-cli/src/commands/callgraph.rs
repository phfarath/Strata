use serde_json::json;
use std::path::{Path, PathBuf};

use strata_core::errors::StrataError;
use strata_memory::{CallEdge, CallGraph, CallGraphAnalyzer, CallType, LanguageKind};

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

/// Executes the `strata callgraph` CLI command.
pub async fn run_callgraph(
    target_path: &str,
    symbol_filter: Option<&str>,
    direction: &str,
    json_output: bool,
    limit: usize,
) -> Result<(), StrataError> {
    let path = Path::new(target_path);
    let analyzer = CallGraphAnalyzer::new();
    let mut all_edges: Vec<CallEdge> = Vec::new();

    if path.is_file() {
        let content = std::fs::read_to_string(path).map_err(|e| {
            StrataError::Internal(format!("Failed to read file {}: {e}", path.display()))
        })?;
        let lang = LanguageKind::from_file_path(target_path);
        let edges = analyzer.analyze_source(&content, lang, target_path)?;
        all_edges.extend(edges);
    } else if path.is_dir() {
        let mut files = Vec::new();
        collect_source_files(path, &mut files);

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
    } else {
        return Err(StrataError::Validation(format!(
            "Path does not exist: {target_path}"
        )));
    }

    let graph = CallGraph::from_edges(all_edges.clone());
    let recursive = graph.detect_recursive_calls();

    let filtered_edges: Vec<CallEdge> = if let Some(sym) = symbol_filter {
        match direction {
            "callers" => graph.callers_of(sym).into_iter().cloned().collect(),
            "callees" => {
                let p = target_path;
                graph.callees_of(p, sym).into_iter().cloned().collect()
            }
            "imports" => graph
                .file_imports(target_path)
                .into_iter()
                .cloned()
                .collect(),
            _ => {
                let callers: Vec<_> = graph.callers_of(sym).into_iter().cloned().collect();
                let callees: Vec<_> = graph
                    .callees_of(target_path, sym)
                    .into_iter()
                    .cloned()
                    .collect();
                let mut merged = callers;
                merged.extend(callees);
                merged
            }
        }
    } else {
        match direction {
            "imports" => all_edges
                .into_iter()
                .filter(|e| e.call_type == CallType::Import)
                .collect(),
            "callers" | "callees" | "both" => all_edges
                .into_iter()
                .filter(|e| e.call_type != CallType::Import)
                .collect(),
            _ => all_edges,
        }
    };

    let total_count = filtered_edges.len();
    let display_edges = if total_count > limit {
        &filtered_edges[..limit]
    } else {
        &filtered_edges[..]
    };

    if json_output {
        let out = json!({
            "target": target_path,
            "symbol": symbol_filter,
            "direction": direction,
            "total_edges": total_count,
            "recursive_calls": recursive,
            "edges": display_edges
        });
        println!("{}", serde_json::to_string_pretty(&out)?);
        return Ok(());
    }

    // CLI Formatted Output
    println!("══════════════════════════════════════════════════════════════════════════");
    println!(" 📞 STRATA NATIVE CALL GRAPH & IMPORT ANALYZER");
    println!("══════════════════════════════════════════════════════════════════════════");
    println!(" Target:    {}", target_path);
    if let Some(sym) = symbol_filter {
        println!(" Symbol:    {}", sym);
    }
    println!(" Direction: {}", direction);
    println!(" Total:     {} edges detected", total_count);

    if !recursive.is_empty() {
        println!("\n ⚠️  RECURSIVE CALL CYCLES ({}):", recursive.len());
        for (f, sym) in &recursive {
            println!("    ↺ {}::{} (direct recursion)", f, sym);
        }
    }

    // Group by file
    let mut by_file: std::collections::HashMap<String, Vec<&CallEdge>> =
        std::collections::HashMap::new();
    for edge in display_edges {
        by_file
            .entry(edge.caller_file.clone())
            .or_default()
            .push(edge);
    }

    println!("\n 📜 CALL & DEPENDENCY HIERARCHY:");
    for (file, file_edges) in by_file {
        println!("  📁 {}", file);
        for edge in file_edges {
            match edge.call_type {
                CallType::Import => {
                    println!(
                        "     📦 [import] line {}: {}",
                        edge.line_number, edge.callee_symbol
                    );
                }
                CallType::ConstructorCall => {
                    println!(
                        "     🏗️  [new]    line {}: {}() -> new {}",
                        edge.line_number, edge.caller_symbol, edge.callee_symbol
                    );
                }
                CallType::MethodCall => {
                    println!(
                        "     🔹 [method] line {}: {}() -> .{}()",
                        edge.line_number, edge.caller_symbol, edge.callee_symbol
                    );
                }
                CallType::MacroCall => {
                    println!(
                        "     ⚡ [macro]  line {}: {}() -> {}()",
                        edge.line_number, edge.caller_symbol, edge.callee_symbol
                    );
                }
                CallType::FunctionCall => {
                    println!(
                        "     📞 [call]   line {}: {}() -> {}()",
                        edge.line_number, edge.caller_symbol, edge.callee_symbol
                    );
                }
            }
        }
    }

    if total_count > limit {
        println!(
            "\n ... truncated (showing {} of {} edges, use --limit to expand)",
            limit, total_count
        );
    }
    println!("══════════════════════════════════════════════════════════════════════════");

    Ok(())
}
