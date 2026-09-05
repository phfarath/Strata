use clap::Args;
use serde::{Deserialize, Serialize};
use std::path::Path;

use strata_core::errors::StrataError;
use strata_memory::{ClusteringConfig, CommunityDetector};

#[derive(Args, Debug, Clone, Serialize, Deserialize)]
pub struct ArchitectureArgs {
    #[arg(
        short,
        long,
        default_value = ".",
        help = "Root workspace or source directory to cluster and analyze"
    )]
    pub path: String,

    #[arg(
        long,
        help = "Workspace identifier (default: current workspace config)"
    )]
    pub workspace: Option<String>,

    #[arg(
        long,
        default_value_t = 1,
        help = "Minimum member symbols/files required to form a distinct cluster"
    )]
    pub min_cluster_size: usize,

    #[arg(
        long,
        default_value_t = 25,
        help = "Maximum LPA convergence iterations"
    )]
    pub max_iterations: usize,

    #[arg(long, help = "Output as raw JSON")]
    pub json: bool,
}

/// Executes the `strata architecture` / `strata cluster` CLI command.
pub async fn run_architecture(args: ArchitectureArgs) -> Result<(), StrataError> {
    let ws_id = args
        .workspace
        .unwrap_or_else(|| crate::config::StrataConfig::resolve_workspace(None));

    let config = ClusteringConfig {
        max_iterations: args.max_iterations,
        min_cluster_size: args.min_cluster_size,
        call_weight: 1.5,
        import_weight: 1.0,
    };

    let detector = CommunityDetector::new(config);
    let summary = detector.detect_from_directory(Path::new(&args.path), &ws_id)?;

    if args.json {
        let json_val = serde_json::to_string_pretty(&summary).map_err(|e| {
            StrataError::Internal(format!(
                "Failed to serialize architecture summary to JSON: {e}"
            ))
        })?;
        println!("{json_val}");
    } else {
        println!("{}", summary.formatted_summary);
    }

    Ok(())
}
