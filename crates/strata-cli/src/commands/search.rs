use anyhow::Result;
use std::sync::Arc;
use strata_core::{
    state::{MemoryType, Scope},
    traits::MemoryEngine,
};

pub struct SearchOptions {
    pub query: String,
    pub limit: usize,
    pub scope: Option<String>,
    pub memory_type: Option<String>,
    pub json: bool,
}

pub async fn run_search(options: SearchOptions, engine: Arc<dyn MemoryEngine>) -> Result<()> {
    let parsed_scope = options
        .scope
        .as_deref()
        .and_then(|s| s.parse::<Scope>().ok());

    let parsed_type = options
        .memory_type
        .as_deref()
        .and_then(|s| s.parse::<MemoryType>().ok());

    let results = engine
        .search(&options.query, parsed_scope.as_ref(), options.limit)
        .await?;

    let filtered: Vec<_> = if let Some(mt) = parsed_type {
        results
            .into_iter()
            .filter(|r| r.memory_type == mt)
            .collect()
    } else {
        results
    };

    if options.json {
        println!("{}", serde_json::to_string_pretty(&filtered)?);
        return Ok(());
    }

    if filtered.is_empty() {
        println!("No memories found matching query: \"{}\"", options.query);
        return Ok(());
    }

    println!(
        "\n🔍 Found {} memories matching \"{}\":\n",
        filtered.len(),
        options.query
    );
    for (i, rec) in filtered.iter().enumerate() {
        let handle = rec.to_handle(None);
        println!(
            "{}. [{}] {} (scope: {}, confidence: {:.2})",
            i + 1,
            handle.memory_type,
            handle.title,
            handle.scope,
            rec.confidence
        );
        println!("   ID: {}", rec.id);
        println!("   Content: {}", rec.content);
        if !rec.tags.is_empty() {
            println!("   Tags: {}", rec.tags.join(", "));
        }
        println!();
    }

    Ok(())
}
