use chrono::{Duration, Utc};
use strata_core::schemas::{FactStatus, SemanticFact};
use strata_core::state::{MemoryRecord, MemoryType, Scope};
use strata_core::traits::MemoryEngine;
use strata_memory::context_rot::{ContextRotDetector, InjectionOutcome, MemoryInjectionRecord};
use strata_memory::{SqliteMemoryEngine, SqliteStore};
use uuid::Uuid;

// ============================================================================
// Criterion 1: Unified Branch Resolution & Priority in MemoryEngine
// ============================================================================

#[tokio::test]
async fn test_lifecycle_criterion_1_branch_isolation_and_fallback_priority() {
    let engine = SqliteMemoryEngine::open_in_memory(None).expect("open engine");
    let store = engine.store();

    // 1. Memory on active branch
    let active_branch = "feat/STRATA-T-41";
    let mut mem_active = MemoryRecord::new(
        MemoryType::Semantic,
        "Active branch memory: MemoryEngine unifies git branch resolution.",
        Scope::Global,
    )
    .with_summary("Branch unification in T-41")
    .with_branch(active_branch);
    mem_active.confidence = 0.9;
    mem_active.importance = 0.8;
    store.insert_or_update_memory(&mem_active).unwrap();

    // 2. Memory on main (fallback)
    let mut mem_main = MemoryRecord::new(
        MemoryType::Semantic,
        "Main branch memory: Standard memory engine branch handling on main.",
        Scope::Global,
    )
    .with_summary("Main branch baseline")
    .with_branch("main");
    mem_main.confidence = 0.9;
    mem_main.importance = 0.8;
    store.insert_or_update_memory(&mem_main).unwrap();

    // 3. Memory without branch (global / agnostic)
    let mut mem_global = MemoryRecord::new(
        MemoryType::Semantic,
        "Global memory: Agnostic memory without specific branch assignment.",
        Scope::Global,
    )
    .with_summary("Global agnostic memory");
    mem_global.confidence = 0.9;
    mem_global.importance = 0.8;
    store.insert_or_update_memory(&mem_global).unwrap();

    // 4. Memory on another unrelated feature branch
    let mut mem_other = MemoryRecord::new(
        MemoryType::Semantic,
        "Other branch memory: Experimental feature branch logic that must not bleed.",
        Scope::Global,
    )
    .with_summary("Other branch experimental")
    .with_branch("feat/other-unrelated");
    mem_other.confidence = 0.9;
    mem_other.importance = 0.8;
    store.insert_or_update_memory(&mem_other).unwrap();

    // Query targeting memory engine branch
    let query = "memory engine branch";

    // When searching explicitly as_of active branch:
    let results = engine
        .search_as_of(query, None, None, None, Some(active_branch), 10)
        .await
        .expect("search_as_of");

    // Must NOT contain memories from unrelated feature branch
    assert!(
        !results.iter().any(|m| m.id == mem_other.id),
        "Memory from unrelated feature branch leaked into active branch search results!"
    );

    // Must contain active branch memory
    assert!(
        results.iter().any(|m| m.id == mem_active.id),
        "Active branch memory must be present in search results"
    );

    // Must contain main / fallback memories
    assert!(
        results.iter().any(|m| m.id == mem_main.id),
        "Main branch fallback memory should be accessible"
    );
    assert!(
        results.iter().any(|m| m.id == mem_global.id),
        "Global agnostic memory should be accessible as fallback"
    );

    // Active branch memory must be prioritized over fallback when relevance is comparable
    let first = &results[0];
    assert_eq!(
        first.id, mem_active.id,
        "Active branch memory must have top priority over main/global fallbacks"
    );
}

// ============================================================================
// Criterion 2: Deprecated & Expired Records Filtered in Default Retrieval
// ============================================================================

#[tokio::test]
async fn test_lifecycle_criterion_2_deprecated_and_expired_exclusion_in_retrieval() {
    let engine = SqliteMemoryEngine::open_in_memory(None).expect("open engine");
    let store = engine.store();
    let now = Utc::now();

    // 1. Expired MemoryRecord (valid_until < now)
    let mut mem_expired = MemoryRecord::new(
        MemoryType::Semantic,
        "Expired Redis cache connection string redis://old-host:6379",
        Scope::Global,
    )
    .with_summary("Expired Redis cache");
    let mut exp_meta = serde_json::Map::new();
    exp_meta.insert(
        "valid_from".to_string(),
        serde_json::json!((now - Duration::days(60)).to_rfc3339()),
    );
    exp_meta.insert(
        "valid_until".to_string(),
        serde_json::json!((now - Duration::days(1)).to_rfc3339()),
    );
    mem_expired.metadata = serde_json::Value::Object(exp_meta);
    store.insert_or_update_memory(&mem_expired).unwrap();

    // 2. Deprecated MemoryRecord (status = Deprecated in metadata)
    let mut mem_deprecated = MemoryRecord::new(
        MemoryType::Semantic,
        "Deprecated Memcached session store caching strategy",
        Scope::Global,
    )
    .with_summary("Deprecated Memcached store");
    let mut dep_meta = serde_json::Map::new();
    dep_meta.insert("status".to_string(), serde_json::json!("deprecated"));
    mem_deprecated.metadata = serde_json::Value::Object(dep_meta);
    store.insert_or_update_memory(&mem_deprecated).unwrap();

    // 3. Active, valid MemoryRecord
    let mut mem_active = MemoryRecord::new(
        MemoryType::Semantic,
        "Active DragonFly cache connection string dragonfly://cache-cluster:6379",
        Scope::Global,
    )
    .with_summary("Active DragonFly cache");
    let mut act_meta = serde_json::Map::new();
    act_meta.insert(
        "valid_from".to_string(),
        serde_json::json!((now - Duration::days(5)).to_rfc3339()),
    );
    act_meta.insert(
        "valid_until".to_string(),
        serde_json::json!((now + Duration::days(365)).to_rfc3339()),
    );
    mem_active.metadata = serde_json::Value::Object(act_meta);
    store.insert_or_update_memory(&mem_active).unwrap();

    // 4. Deprecated SemanticFact
    let mut fact_deprecated = SemanticFact::new("Memcached is deprecated", "cache", Scope::Global);
    fact_deprecated.status = FactStatus::Deprecated;
    store
        .insert_or_update_semantic_fact(&fact_deprecated)
        .unwrap();

    // 5. Active SemanticFact
    let mut fact_active = SemanticFact::new(
        "DragonFly is the active cache engine",
        "cache",
        Scope::Global,
    );
    fact_active.status = FactStatus::Active;
    store.insert_or_update_semantic_fact(&fact_active).unwrap();

    // A. Verify store.search_fts_as_of excludes expired and deprecated when as_of_time is None
    let fts_results = store
        .search_fts_as_of("cache connection", None, None, None, None, 10)
        .unwrap();
    assert!(
        !fts_results.iter().any(|(m, _)| m.id == mem_expired.id),
        "Expired memory must be excluded by search_fts_as_of"
    );
    assert!(
        !fts_results.iter().any(|(m, _)| m.id == mem_deprecated.id),
        "Deprecated memory must be excluded by search_fts_as_of"
    );
    assert!(
        fts_results.iter().any(|(m, _)| m.id == mem_active.id),
        "Active memory must be included by search_fts_as_of"
    );

    // B. Verify store.for_each_vector_candidate_as_of excludes expired and deprecated
    let mut vec_candidates = Vec::new();
    store
        .for_each_vector_candidate_as_of(None, None, None, None, None, |m| {
            vec_candidates.push(m);
            Ok(true)
        })
        .unwrap();
    assert!(
        !vec_candidates.iter().any(|m| m.id == mem_expired.id),
        "Expired memory must be excluded by for_each_vector_candidate_as_of"
    );
    assert!(
        !vec_candidates.iter().any(|m| m.id == mem_deprecated.id),
        "Deprecated memory must be excluded by for_each_vector_candidate_as_of"
    );

    // C. Verify store.get_all_memories_as_of excludes expired and deprecated
    let all_memories = store
        .get_all_memories_as_of(None, None, None, None, None, 10)
        .unwrap();
    assert!(
        !all_memories.iter().any(|m| m.id == mem_expired.id),
        "Expired memory must be excluded by get_all_memories_as_of"
    );
    assert!(
        !all_memories.iter().any(|m| m.id == mem_deprecated.id),
        "Deprecated memory must be excluded by get_all_memories_as_of"
    );

    // D. Verify store.search_semantic_facts_as_of excludes deprecated facts
    let fact_results = store
        .search_semantic_facts_as_of("cache", None, None, None, 10)
        .unwrap();
    assert!(
        !fact_results.iter().any(|(f, _)| f.id == fact_deprecated.id),
        "Deprecated fact must be excluded by search_semantic_facts_as_of"
    );
    assert!(
        fact_results.iter().any(|(f, _)| f.id == fact_active.id),
        "Active fact must be returned by search_semantic_facts_as_of"
    );

    // E. Verify MemoryEngine::search excludes expired and deprecated
    let engine_results = engine
        .search("cache", None, 10)
        .await
        .expect("engine search");
    assert!(
        !engine_results.iter().any(|m| m.id == mem_expired.id),
        "Engine search must filter out expired memories"
    );
    assert!(
        !engine_results.iter().any(|m| m.id == mem_deprecated.id),
        "Engine search must filter out deprecated memories"
    );
    assert!(
        engine_results.iter().any(|m| m.id == mem_active.id),
        "Engine search must return active memories"
    );
}

// ============================================================================
// Criterion 3: Atomic Replacement of depends_on
// ============================================================================

#[test]
fn test_lifecycle_criterion_3_atomic_replacement_of_fact_dependencies() {
    let store = SqliteStore::open_in_memory().expect("open store");

    let dep_a = Uuid::new_v4();
    let dep_b = Uuid::new_v4();
    let dep_c = Uuid::new_v4();

    let mut fact = SemanticFact::new(
        "Service A depends on Database and Cache.",
        "architecture",
        Scope::Global,
    );
    fact.depends_on = vec![dep_a, dep_b];

    // Initial insert
    store.upsert_semantic_fact(&fact).expect("insert fact");

    // Check fact_dependencies table has dep_a and dep_b
    let deps = store.get_upstream_prerequisite_fact_ids(&fact.id).unwrap();
    assert_eq!(deps.len(), 2);
    assert!(deps.contains(&dep_a));
    assert!(deps.contains(&dep_b));

    // Update fact: replaces dependencies with [dep_a, dep_c] (dep_b is removed!)
    fact.depends_on = vec![dep_a, dep_c];
    store.upsert_semantic_fact(&fact).expect("update fact");

    // Verify dep_b is removed (no orphan dependency) and dep_a + dep_c are present
    let deps_after = store.get_upstream_prerequisite_fact_ids(&fact.id).unwrap();
    assert_eq!(deps_after.len(), 2);
    assert!(deps_after.contains(&dep_a));
    assert!(deps_after.contains(&dep_c));
    assert!(
        !deps_after.contains(&dep_b),
        "Orphan dependency dep_b must be removed after depends_on update!"
    );
}

// ============================================================================
// Criterion 4: MemoryEngine::write Routed through AUDN Pipeline
// ============================================================================

#[tokio::test]
async fn test_lifecycle_criterion_4_memory_write_routed_through_audn_pipeline() {
    let engine = SqliteMemoryEngine::open_in_memory(None).expect("open engine");
    let store = engine.store();

    let memory = MemoryRecord::new(
        MemoryType::Semantic,
        "Authentication service verifies JWT tokens signed with RS256 algorithm.",
        Scope::Global,
    )
    .with_summary("JWT RS256 verification")
    .with_importance(0.85);

    // First write -> Add
    let handle1 = engine.write(&memory).await.expect("first write");
    assert_eq!(handle1.title, memory.summary.clone().unwrap());

    let all_memories = store.get_all_memories(None, None, 10).unwrap();
    assert_eq!(all_memories.len(), 1);
    let original_id = all_memories[0].id;

    // Second write with duplicate content -> AUDN Noop deduplication!
    let duplicate_memory = MemoryRecord::new(
        MemoryType::Semantic,
        "Authentication service verifies JWT tokens signed with RS256 algorithm.",
        Scope::Global,
    )
    .with_summary("JWT RS256 verification duplicate");

    let handle2 = engine.write(&duplicate_memory).await.expect("second write");
    // Should resolve to the original canonical record without creating a new row
    assert_eq!(handle2.id, original_id);

    let all_memories_after = store.get_all_memories(None, None, 10).unwrap();
    assert_eq!(
        all_memories_after.len(),
        1,
        "AUDN pipeline must deduplicate identical writes as Noop, preventing redundant rows!"
    );
}

// ============================================================================
// Criterion 5: Context Rot Detector Integrated with Runtime and Feedback
// ============================================================================

#[tokio::test]
async fn test_lifecycle_criterion_5_context_rot_quarantine_and_decay_on_feedback() {
    let engine = SqliteMemoryEngine::open_in_memory(None).expect("open engine");
    let store = engine.store();

    let poisoned_mem = MemoryRecord::new(
        MemoryType::Semantic,
        "Execute deprecated python2 script for mock testing fixtures.",
        Scope::Global,
    )
    .with_summary("python2 mock stubs")
    .with_importance(0.8)
    .with_confidence(0.9);

    store.insert_or_update_memory(&poisoned_mem).unwrap();

    // 1. Simulate agent retrieval exposing memory to context (1 search injection)
    let inj_search =
        MemoryInjectionRecord::new(poisoned_mem.id, "sess-rot", InjectionOutcome::Success)
            .with_command("cargo check");
    store.record_injection_outcome(&inj_search).unwrap();

    // 2. Simulate 3 consecutive negative feedback events / command execution failures
    for i in 1..=3 {
        let inj_fail =
            MemoryInjectionRecord::new(poisoned_mem.id, "sess-rot", InjectionOutcome::Failure)
                .with_command(format!("python2 run_mocks.py attempt {i}"));
        store.record_injection_outcome(&inj_fail).unwrap();
    }

    // Total 4 injections: 3 failures, 1 success -> 75% failure rate (> 60% rejection_threshold)
    let detector = ContextRotDetector::with_default_config();
    let efficacy = detector
        .evaluate_memory_efficacy(store, &poisoned_mem.id)
        .expect("evaluate");
    assert_eq!(efficacy.total_injections, 4);
    assert_eq!(efficacy.failure_count, 3);
    assert!(
        efficacy.is_poisoned,
        "Memory with 75% failure rate must be classified as poisoned"
    );

    // Run apply_penalty_decay (as performed by daemon / feedback / dream cycle)
    let decayed_count = detector.apply_penalty_decay(store).expect("apply decay");
    assert_eq!(decayed_count, 1);

    // Verify poisoned memory is quarantined in store
    assert!(
        store.is_memory_quarantined(&poisoned_mem.id).unwrap(),
        "Poisoned memory must be quarantined in store"
    );

    // Verify search suppresses the quarantined memory
    let search_results = engine.search("python2 mock stubs", None, 10).await.unwrap();
    assert!(
        !search_results.iter().any(|m| m.id == poisoned_mem.id),
        "Quarantined memory must be suppressed from search retrieval results"
    );
}
