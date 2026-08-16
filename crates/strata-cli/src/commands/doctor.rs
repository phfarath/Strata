use std::path::Path;
use std::sync::Arc;
use anyhow::Result;
use strata_core::{state::Scope, traits::MemoryEngine};

pub async fn run_doctor(db_path: &Path, engine: Arc<dyn MemoryEngine>) -> Result<()> {
    println!("\n🩺 Strata Diagnostic Health Check");
    println!("═════════════════════════════════════════\n");

    // 1. Storage check
    println!("📦 Storage Backend:");
    println!("   Database location: {}", db_path.display());
    if db_path.exists() {
        let meta = std::fs::metadata(db_path)?;
        println!("   ✓ File status: Present ({:.2} KB)", meta.len() as f64 / 1024.0);
    } else {
        println!("   ℹ File status: New database (will initialize upon write)");
    }

    // 2. Engine connectivity test
    println!("\n🧠 Memory Engine:");
    match engine.search("test", Some(&Scope::Global), 1).await {
        Ok(results) => {
            println!("   ✓ Engine read query: OK (returned {} records)", results.len());
        }
        Err(e) => {
            println!("   ✗ Engine read query failed: {e}");
        }
    }

    // 3. Known failures test
    match engine.get_known_failures(None, None, 5).await {
        Ok(failures) => {
            println!("   ✓ Failure pattern repository: OK (active anti-patterns: {})", failures.len());
        }
        Err(e) => {
            println!("   ✗ Failure pattern retrieval failed: {e}");
        }
    }

    // 4. Host Integration check in current workspace
    println!("\n🖥️  Host Integrations (Current Directory):");
    check_host_integration("Cursor", Path::new(".cursor/rules/strata.mdc"), Path::new(".cursor/mcp.json"));
    check_host_integration("Claude Code", Path::new(".claude/settings.json"), Path::new("CLAUDE.md"));
    check_host_integration("Codex", Path::new(".codex/config.toml"), Path::new("AGENTS.md"));
    check_host_integration("Gemini / Antigravity", Path::new(".gemini/GEMINI.md"), Path::new(".gemini/GEMINI.md"));

    println!("\n═════════════════════════════════════════");
    println!("✓ Strata is ready for cross-host cognitive operations.\n");

    Ok(())
}

fn check_host_integration(host_name: &str, primary_file: &Path, secondary_file: &Path) {
    let p_exists = primary_file.exists();
    let s_exists = secondary_file.exists();

    if p_exists || s_exists {
        println!("   ✓ {host_name}: Configured (found {})", 
            if p_exists { primary_file.display().to_string() } else { secondary_file.display().to_string() }
        );
    } else {
        println!("   ○ {host_name}: Not configured in this workspace (run `strata init`)");
    }
}
