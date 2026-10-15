use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::time::Instant;

use strata_core::a2a::{AgentCard, DomainAuthority, TransactiveQuery};
use strata_core::errors::StrataError;
use strata_memory::SqliteMemoryEngine;

/// Evaluation scenario measuring Transactive Memory Directory capabilities:
/// capability announcement, hierarchical domain routing, active lease precedence,
/// and contextual query routing across peer agents (< 5ms latency).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct A2aTransactiveDirectoryEvalResult {
    pub capability_announcement_verified: bool,
    pub hierarchical_prefix_matching_verified: bool,
    pub specificity_precedence_verified: bool,
    pub active_lease_precedence_verified: bool,
    pub query_routing_and_inbox_delivery_verified: bool,
    pub evaluation_duration_micros: u128,
    pub is_sub_10ms: bool,
}

pub struct A2aTransactiveDirectoryEval;

impl A2aTransactiveDirectoryEval {
    pub async fn run_eval() -> Result<A2aTransactiveDirectoryEvalResult, StrataError> {
        let temp_dir = std::env::temp_dir().join("strata_transactive_dir_eval_test");
        let _ = std::fs::create_dir_all(&temp_dir);
        let db_path = temp_dir.join("transactive_eval.db");
        let _ = std::fs::remove_file(&db_path);

        let engine = SqliteMemoryEngine::open(&db_path, None)?;
        let coord = engine.stigmergy();

        let start = Instant::now();

        // 1. Multi-Agent Capability Announcement
        let card1 = AgentCard::new("agent-memory-specialist", "claude-code")
            .with_domain_scopes(vec!["crates/strata-memory".into()])
            .with_confidence(0.90);
        let card2 = AgentCard::new("agent-jtms-specialist", "cursor")
            .with_domain_scopes(vec!["crates/strata-memory/src/jtms.rs".into()])
            .with_confidence(0.98);
        let card3 = AgentCard::new("agent-cli-specialist", "antigravity")
            .with_domain_scopes(vec!["crates/strata-cli".into()])
            .with_confidence(0.85);

        coord.register_agent_card(&card1)?;
        coord.register_agent_card(&card2)?;
        coord.register_agent_card(&card3)?;

        let all_cards = coord.list_agent_cards()?;
        let capability_announcement_verified = all_cards.len() == 3
            && all_cards.iter().any(|c| c.agent_id == "agent-memory-specialist")
            && all_cards.iter().any(|c| c.agent_id == "agent-jtms-specialist");

        // Register domain authorities
        coord.set_domain_authority(&DomainAuthority::new(
            "crates/strata-memory",
            "agent-memory-specialist",
            0.90,
        ))?;
        coord.set_domain_authority(&DomainAuthority::new(
            "crates/strata-memory/src/jtms.rs",
            "agent-jtms-specialist",
            0.98,
        ))?;
        coord.set_domain_authority(&DomainAuthority::new(
            "crates/strata-cli",
            "agent-cli-specialist",
            0.85,
        ))?;

        // 2. Hierarchical Prefix Matching:
        // A file "crates/strata-memory/src/store.rs" has no specific authority,
        // so it must hierarchically resolve to "crates/strata-memory" -> "agent-memory-specialist".
        let resolved_store = coord.resolve_domain_authority("crates/strata-memory/src/store.rs")?;
        let hierarchical_prefix_matching_verified = match &resolved_store {
            Some(auth) => {
                auth.authoritative_agent == "agent-memory-specialist"
                    && auth.domain == "crates/strata-memory"
            }
            None => false,
        };

        // 3. Specificity Precedence:
        // "crates/strata-memory/src/jtms.rs" has both parent "crates/strata-memory" and specific authority.
        // It must resolve to the longer, more specific domain "crates/strata-memory/src/jtms.rs" -> "agent-jtms-specialist".
        let resolved_jtms = coord.resolve_domain_authority("crates/strata-memory/src/jtms.rs")?;
        let specificity_precedence_verified = match &resolved_jtms {
            Some(auth) => {
                auth.authoritative_agent == "agent-jtms-specialist"
                    && auth.domain == "crates/strata-memory/src/jtms.rs"
            }
            None => false,
        };

        // 4. Active Lease Precedence:
        // Agent "agent-core-worker" acquires an exclusive lease on "crates/strata-cli/src/main.rs"
        let _acq = coord.acquire_lease(
            "crates/strata-cli/src/main.rs",
            "agent-core-worker",
            60,
            Some("active refactoring"),
        )?;

        let resolved_cli_file = coord.resolve_domain_authority("crates/strata-cli/src/main.rs")?;
        let active_lease_precedence_verified = match &resolved_cli_file {
            Some(auth) => {
                // Must be the lease holder and have lease_held == true
                auth.authoritative_agent == "agent-core-worker" && auth.lease_held
            }
            None => false,
        };

        // 5. Query Routing & Inbox Delivery:
        // Simulate an agent querying "crates/strata-memory/src/jtms.rs"
        let target_authority = coord.resolve_domain_authority("crates/strata-memory/src/jtms.rs")?
            .expect("must resolve authority");
        let query = TransactiveQuery::new(
            "qry-eval-1",
            "agent-cli-specialist",
            "crates/strata-memory/src/jtms.rs",
            "How does contradiction propagation work in JTMS?",
        );

        // Record transactive query in recipient's inbox
        let msg = strata_core::a2a::A2aMessage::direct(
            "msg-eval-1",
            &query.from_agent,
            &target_authority.authoritative_agent,
            &query.query_text,
        ).with_kind("transactive_query");
        coord.save_a2a_message(&msg)?;

        let inbox = coord.get_a2a_inbox("agent-jtms-specialist", true)?;
        let query_routing_and_inbox_delivery_verified = inbox.len() == 1
            && inbox[0].message_id == "msg-eval-1"
            && inbox[0].from_agent == "agent-cli-specialist";

        let elapsed = start.elapsed();
        let elapsed_micros = elapsed.as_micros();

        let _ = std::fs::remove_dir_all(&temp_dir);

        Ok(A2aTransactiveDirectoryEvalResult {
            capability_announcement_verified,
            hierarchical_prefix_matching_verified,
            specificity_precedence_verified,
            active_lease_precedence_verified,
            query_routing_and_inbox_delivery_verified,
            evaluation_duration_micros: elapsed_micros,
            is_sub_10ms: elapsed_micros < 10_000,
        })
    }
}

pub async fn run_a2a_transactive_directory_scenario() -> Result<()> {
    println!("\n--------------------------------------------------------");
    println!("Scenario: A2A Transactive Memory Directory & Specialization Routing");
    println!("--------------------------------------------------------");

    let result = A2aTransactiveDirectoryEval::run_eval().await?;

    println!(
        "✓ Capability Cards Announced & Discovered:  {}",
        result.capability_announcement_verified
    );
    println!(
        "✓ Hierarchical Prefix Matching Verified:    {}",
        result.hierarchical_prefix_matching_verified
    );
    println!(
        "✓ Specificity Precedence Verified:          {}",
        result.specificity_precedence_verified
    );
    println!(
        "✓ Active Lease Precedence Verified:         {}",
        result.active_lease_precedence_verified
    );
    println!(
        "✓ Transactive Query & Inbox Delivery:       {}",
        result.query_routing_and_inbox_delivery_verified
    );
    println!(
        "⏱ Total Duration:                            {:.2} ms (sub-10ms: {})",
        result.evaluation_duration_micros as f64 / 1000.0,
        result.is_sub_10ms
    );

    assert!(result.capability_announcement_verified);
    assert!(result.hierarchical_prefix_matching_verified);
    assert!(result.specificity_precedence_verified);
    assert!(result.active_lease_precedence_verified);
    assert!(result.query_routing_and_inbox_delivery_verified);

    Ok(())
}
