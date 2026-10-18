use chrono::{Duration, Utc};
use strata_core::state::{MemoryRecord, MemoryTier, MemoryType, Scope};
use strata_memory::embedding::{EmbeddingProvider, MockEmbeddingProvider};
use strata_memory::retrieval::{HybridRanker, HybridRankerConfig};
use strata_memory::SqliteStore;

#[tokio::test]
async fn test_retrieval_does_not_prematurely_cut_off_old_memories_with_100_distractors() {
    let store = SqliteStore::open_in_memory().expect("open store");
    let embedder = MockEmbeddingProvider::default();
    let ranker = HybridRanker::with_default_config();

    let now = Utc::now();
    let old_time = now - Duration::days(180);

    // 1. Create 1 old, highly relevant memory (created 180 days ago)
    let mut old_memory = MemoryRecord::new(
        MemoryType::Semantic,
        "PostgreSQL connection pooling and pgbouncer timeout configuration parameters".to_string(),
        Scope::Global,
    )
    .with_summary("pgbouncer pool configuration".to_string())
    .with_importance(0.8)
    .with_confidence(0.9);
    old_memory.created_at = old_time;
    old_memory.updated_at = old_time;
    old_memory.tier = MemoryTier::Peripheral;
    old_memory.embedding = Some(embedder.embed_text(&old_memory.content).await.unwrap());
    store.insert_or_update_memory(&old_memory).unwrap();

    // 2. Insert 120 recent distractor memories (created today/yesterday)
    for i in 1..=120 {
        let recent_time = now - Duration::hours(i as i64 % 24);
        let mut distractor = MemoryRecord::new(
            MemoryType::Semantic,
            format!("Frontend UI button styling variant #{i} with flexbox and css margin padding"),
            Scope::Global,
        )
        .with_summary(format!("UI button variant {i}"))
        .with_importance(0.5)
        .with_confidence(0.7);
        distractor.created_at = recent_time;
        distractor.updated_at = recent_time;
        distractor.tier = MemoryTier::Peripheral;
        distractor.embedding = Some(embedder.embed_text(&distractor.content).await.unwrap());
        store.insert_or_update_memory(&distractor).unwrap();
    }

    // 3. Query targeting semantic match for the old memory
    let query = "PostgreSQL connection pooling pgbouncer";

    // 4. Test pure vector retrieval (bm25_weight = 0.0)
    // Under the bug, vector candidates are limited to the top 40 most recent memories,
    // so old_memory is never evaluated and will be absent from vector results!
    let vector_only_ranker = HybridRanker::new(HybridRankerConfig {
        bm25_weight: 0.0,
        vector_weight: 1.0,
        graph_weight: 0.0,
        ..HybridRankerConfig::default()
    });
    let vec_results = vector_only_ranker
        .retrieve(&store, Some(&embedder), query, Some(&Scope::Global), None, 5)
        .await
        .expect("retrieve");
    assert!(
        vec_results.iter().any(|m| m.id == old_memory.id),
        "Old relevant memory must be present in vector retrieval results, but was omitted by premature recency cutoff! Got: {:?}",
        vec_results.iter().map(|m| (&m.id, &m.summary)).collect::<Vec<_>>()
    );

    // 5. Test hybrid retrieval with default ranker
    let results = ranker
        .retrieve(&store, Some(&embedder), query, Some(&Scope::Global), None, 5)
        .await
        .expect("retrieve");

    // The old memory must NOT have been cut off by a premature chronological LIMIT!
    // Definition of done: The old exact match must appear in the top 3 retrieved results.
    let found_old_memory = results.iter().take(3).any(|m| m.id == old_memory.id);
    assert!(
        found_old_memory,
        "Old relevant memory must appear in top 3 despite 120 recent distractors, got: {:?}",
        results.iter().map(|m| (&m.id, &m.summary)).collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn test_continuous_recency_weighting_balances_semantic_and_temporal_decay() {
    let store = SqliteStore::open_in_memory().expect("open store");
    let embedder = MockEmbeddingProvider::default();

    let now = Utc::now();
    let old_time = now - Duration::days(90);
    let recent_time = now - Duration::minutes(15);

    // Memory A: Old (90 days ago)
    let mut mem_old = MemoryRecord::new(
        MemoryType::Semantic,
        "Docker container networking DNS resolution bridge interface".to_string(),
        Scope::Global,
    )
    .with_summary("Docker network DNS old".to_string())
    .with_importance(0.6)
    .with_confidence(0.8);
    mem_old.created_at = old_time;
    mem_old.updated_at = old_time;
    mem_old.tier = MemoryTier::Peripheral;
    mem_old.embedding = Some(embedder.embed_text(&mem_old.content).await.unwrap());
    store.insert_or_update_memory(&mem_old).unwrap();

    // Memory B: Recent (15 mins ago), same content & quality
    let mut mem_recent = MemoryRecord::new(
        MemoryType::Semantic,
        "Docker container networking DNS resolution bridge interface".to_string(),
        Scope::Global,
    )
    .with_summary("Docker network DNS recent".to_string())
    .with_importance(0.6)
    .with_confidence(0.8);
    mem_recent.created_at = recent_time;
    mem_recent.updated_at = recent_time;
    mem_recent.tier = MemoryTier::Peripheral;
    mem_recent.embedding = Some(embedder.embed_text(&mem_recent.content).await.unwrap());
    store.insert_or_update_memory(&mem_recent).unwrap();

    let query = "Docker container networking DNS";

    // 1. With continuous recency weighting (default recency_weight > 0.0)
    let ranker_with_recency = HybridRanker::with_default_config();
    let results = ranker_with_recency
        .retrieve(&store, Some(&embedder), query, Some(&Scope::Global), None, 5)
        .await
        .expect("retrieve");

    assert_eq!(results.len(), 2);
    assert_eq!(
        results[0].id, mem_recent.id,
        "Recent memory should be ranked higher than older memory when semantic relevance is identical"
    );
    assert_eq!(
        results[1].id, mem_old.id,
        "Old memory should still be retained in results directly following recent memory"
    );
}
