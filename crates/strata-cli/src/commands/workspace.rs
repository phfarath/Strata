use std::path::Path;
use serde_json::json;

use strata_core::errors::StrataError;
use strata_memory::WorkspaceBoundaryDetector;

/// Arguments for `strata workspace` command.
#[derive(clap::Args, Debug, Clone)]
pub struct WorkspaceArgs {
    #[arg(long, default_value = ".", help = "Root directory to scan for workspace boundaries")]
    pub path: String,

    #[arg(long, help = "Optional file path to resolve to its owner package")]
    pub file: Option<String>,

    #[arg(long, help = "Output as raw JSON")]
    pub json: bool,
}

/// Executes the `strata workspace` CLI command.
pub async fn run_workspace(args: WorkspaceArgs) -> Result<(), StrataError> {
    let root = Path::new(&args.path);
    let boundary = WorkspaceBoundaryDetector::detect(root)?;

    let mut resolved_package = None;
    let mut hierarchical_scopes = Vec::new();

    if let Some(ref f) = args.file {
        resolved_package = boundary.find_package_for_file(f).cloned();
        hierarchical_scopes = boundary
            .get_hierarchical_scopes(f)
            .into_iter()
            .map(|s| s.to_string())
            .collect();
    }

    if args.json {
        let out = json!({
            "root_path": boundary.root_path,
            "workspace_type": boundary.workspace_type.to_string(),
            "packages_count": boundary.packages.len(),
            "packages": boundary.packages,
            "target_file": args.file,
            "resolved_package": resolved_package,
            "hierarchical_scopes": hierarchical_scopes
        });
        println!("{}", serde_json::to_string_pretty(&out)?);
        return Ok(());
    }

    println!("══════════════════════════════════════════════════════════════════════════");
    println!(" 🏢 STRATA MONOREPO & WORKSPACE BOUNDARY DETECTOR");
    println!("══════════════════════════════════════════════════════════════════════════");
    println!(" Root Path:       {}", boundary.root_path);
    println!(" Workspace Type:  {}", boundary.workspace_type);
    println!(" Member Packages: {} packages detected\n", boundary.packages.len());

    println!(" 📦 DISCOVERED PACKAGES:");
    for pkg in &boundary.packages {
        let deps_str = if pkg.internal_dependencies.is_empty() {
            "".to_string()
        } else {
            format!(" -> deps: [{}]", pkg.internal_dependencies.join(", "))
        };
        println!("  • {:<20} [{}] @ {}{}", pkg.name, pkg.package_type, pkg.root_path, deps_str);
    }

    if let Some(ref pkg) = resolved_package {
        println!("\n 🎯 FILE BOUNDARY RESOLUTION:");
        println!("  • File:                 {}", args.file.as_deref().unwrap_or(""));
        println!("  • Owning Package:       {}", pkg.name);
        println!("  • Hierarchical Scopes:  {}", hierarchical_scopes.join(" -> "));
    }

    println!("══════════════════════════════════════════════════════════════════════════");

    Ok(())
}
