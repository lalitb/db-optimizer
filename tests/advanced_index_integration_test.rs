// Integration tests for Advanced Index Intelligence
// These tests require a real PostgreSQL database
// Run with: TEST_DATABASE_URL=postgresql://user:pass@localhost/test cargo test --test advanced_index_integration_test -- --ignored

use pg_query_advisor::analyzer::advanced::{
    AdvancedIndexAnalyzer,
    column_stats::ColumnStatsCollector,
    index_type_selector::IndexTypeSelector,
};
use sqlx::PgPool;

async fn setup_test_db() -> PgPool {
    let database_url = std::env::var("TEST_DATABASE_URL")
        .unwrap_or_else(|_| "postgresql://postgres:postgres@localhost/test_pg_advisor".to_string());
    
    PgPool::connect(&database_url)
        .await
        .expect("Failed to connect to test database")
}

async fn create_test_tables(pool: &PgPool) {
    // Create test tables with various data types and patterns
    
    // Table 1: Events with sequential timestamp (BRIN candidate)
    sqlx::query(r#"
        DROP TABLE IF EXISTS test_events CASCADE;
        CREATE TABLE test_events (
            id SERIAL PRIMARY KEY,
            event_type VARCHAR(50),
            created_at TIMESTAMP NOT NULL DEFAULT NOW(),
            metadata JSONB,
            tags TEXT[]
        );
    "#)
    .execute(pool)
    .await
    .expect("Failed to create test_events table");
    
    // Insert sequential data
    sqlx::query(r#"
        INSERT INTO test_events (event_type, created_at, metadata, tags)
        SELECT 
            CASE (random() * 3)::int
                WHEN 0 THEN 'login'
                WHEN 1 THEN 'pageview'
                WHEN 2 THEN 'purchase'
                ELSE 'other'
            END,
            NOW() + (i || ' seconds')::interval,
            jsonb_build_object('user_id', i, 'session', gen_random_uuid()::text),
            ARRAY['tag' || (i % 10)::text]
        FROM generate_series(1, 10000) i;
    "#)
    .execute(pool)
    .await
    .expect("Failed to insert test data");
    
    // Table 2: Orders with JSONB metadata (GIN candidate)
    sqlx::query(r#"
        DROP TABLE IF EXISTS test_orders CASCADE;
        CREATE TABLE test_orders (
            id SERIAL PRIMARY KEY,
            user_id INTEGER,
            status VARCHAR(20),
            metadata JSONB,
            items JSONB[]
        );
    "#)
    .execute(pool)
    .await
    .expect("Failed to create test_orders table");
    
    // Insert data with skewed status (95% "completed" for partial index test)
    sqlx::query(r#"
        INSERT INTO test_orders (user_id, status, metadata, items)
        SELECT 
            (random() * 1000)::int,
            CASE 
                WHEN random() < 0.95 THEN 'completed'
                WHEN random() < 0.97 THEN 'pending'
                ELSE 'cancelled'
            END,
            jsonb_build_object(
                'total', (random() * 1000)::numeric(10,2),
                'discount', (random() * 100)::numeric(10,2),
                'shipping_address', jsonb_build_object(
                    'city', 'City' || (random() * 100)::int,
                    'zip', '12345'
                )
            ),
            ARRAY[
                jsonb_build_object('sku', 'SKU' || (random() * 100)::int, 'qty', (random() * 10)::int)
            ]
        FROM generate_series(1, 10000) i;
    "#)
    .execute(pool)
    .await
    .expect("Failed to insert test orders");
    
    // Table 3: Products with text search (GIN trigram candidate)
    sqlx::query(r#"
        DROP TABLE IF EXISTS test_products CASCADE;
        CREATE TABLE test_products (
            id SERIAL PRIMARY KEY,
            name TEXT,
            description TEXT,
            price NUMERIC(10,2),
            category VARCHAR(50)
        );
    "#)
    .execute(pool)
    .await
    .expect("Failed to create test_products table");
    
    sqlx::query(r#"
        INSERT INTO test_products (name, description, price, category)
        SELECT 
            'Product ' || i || ' ' || md5(random()::text),
            'Description for product ' || i,
            (random() * 1000)::numeric(10,2),
            'Category' || (i % 10)::text
        FROM generate_series(1, 1000) i;
    "#)
    .execute(pool)
    .await
    .expect("Failed to insert test products");
    
    // Table 4: High cardinality UUIDs (HASH candidate)
    sqlx::query(r#"
        DROP TABLE IF EXISTS test_sessions CASCADE;
        CREATE TABLE test_sessions (
            id SERIAL PRIMARY KEY,
            session_id UUID UNIQUE,
            user_id INTEGER,
            last_activity TIMESTAMP
        );
    "#)
    .execute(pool)
    .await
    .expect("Failed to create test_sessions table");
    
    sqlx::query(r#"
        INSERT INTO test_sessions (session_id, user_id, last_activity)
        SELECT 
            gen_random_uuid(),
            (random() * 10000)::int,
            NOW() - (random() * interval '30 days')
        FROM generate_series(1, 5000) i;
    "#)
    .execute(pool)
    .await
    .expect("Failed to insert test sessions");
    
    // Run ANALYZE to populate pg_stats
    sqlx::query("ANALYZE;")
        .execute(pool)
        .await
        .expect("Failed to run ANALYZE");
}

#[tokio::test]
#[ignore] // Requires TEST_DATABASE_URL
async fn test_brin_detection_for_sequential_timestamps() {
    let pool = setup_test_db().await;
    create_test_tables(&pool).await;
    
    let mut collector = ColumnStatsCollector::new(pool.clone());
    let stats = collector
        .collect_column_stats("public", "test_events", "created_at")
        .await
        .expect("Failed to collect stats");
    
    // Verify sequential pattern detection
    assert!(stats.correlation.abs() > 0.8, "Expected high correlation for sequential data");
    assert!(stats.is_sequential, "Expected is_sequential flag to be true");
    
    // Test index type selection
    let selector = IndexTypeSelector::new();
    let rec = selector.select_index_type(&stats, &[]);
    
    assert_eq!(
        rec.index_type,
        pg_query_advisor::analyzer::advanced::index_type_selector::IndexType::BRIN,
        "Expected BRIN index for sequential timestamp"
    );
    assert!(rec.estimated_size_mb < 10.0, "BRIN should be < 10MB for 10k rows");
    assert!(rec.confidence_score > 0.85, "Expected high confidence for BRIN");
    
    println!("✅ BRIN Detection Test Passed");
    println!("   Correlation: {:.2}", stats.correlation);
    println!("   Estimated Size: {:.2}MB", rec.estimated_size_mb);
    println!("   DDL: {}", rec.ddl);
}

#[tokio::test]
#[ignore]
async fn test_gin_detection_for_jsonb() {
    let pool = setup_test_db().await;
    create_test_tables(&pool).await;
    
    let mut collector = ColumnStatsCollector::new(pool.clone());
    let stats = collector
        .collect_column_stats("public", "test_orders", "metadata")
        .await
        .expect("Failed to collect stats");
    
    // Verify JSONB detection
    assert_eq!(
        stats.data_type,
        pg_query_advisor::analyzer::advanced::column_stats::PostgresType::Jsonb,
        "Expected JSONB type"
    );
    
    // Test index type selection
    let selector = IndexTypeSelector::new();
    let rec = selector.select_index_type(&stats, &[]);
    
    assert_eq!(
        rec.index_type,
        pg_query_advisor::analyzer::advanced::index_type_selector::IndexType::GIN,
        "Expected GIN index for JSONB"
    );
    assert_eq!(
        rec.opclass,
        Some("jsonb_path_ops".to_string()),
        "Expected jsonb_path_ops operator class"
    );
    assert!(rec.confidence_score > 0.9, "Expected high confidence for GIN");
    
    println!("✅ GIN Detection Test Passed");
    println!("   Data Type: {:?}", stats.data_type);
    println!("   Operator Class: {:?}", rec.opclass);
    println!("   DDL: {}", rec.ddl);
}

#[tokio::test]
#[ignore]
async fn test_partial_index_for_skewed_data() {
    let pool = setup_test_db().await;
    create_test_tables(&pool).await;
    
    let mut collector = ColumnStatsCollector::new(pool.clone());
    let stats = collector
        .collect_column_stats("public", "test_orders", "status")
        .await
        .expect("Failed to collect stats");
    
    // Verify low cardinality
    assert!(stats.cardinality_ratio < 0.01, "Expected low cardinality");
    
    // Check if most common value is dominant
    if !stats.most_common_freqs.is_empty() {
        assert!(
            stats.most_common_freqs[0] > 0.9,
            "Expected dominant value (>90%)"
        );
    }
    
    // Test index type selection
    let selector = IndexTypeSelector::new();
    let rec = selector.select_index_type(&stats, &[]);
    
    assert_eq!(
        rec.index_type,
        pg_query_advisor::analyzer::advanced::index_type_selector::IndexType::BTree,
        "Expected BTREE for low cardinality"
    );
    
    // Should recommend partial index
    assert!(
        rec.where_clause.is_some(),
        "Expected WHERE clause for partial index"
    );
    
    if let Some(where_clause) = &rec.where_clause {
        assert!(
            where_clause.contains("completed"),
            "Expected partial index to exclude 'completed'"
        );
    }
    
    println!("✅ Partial Index Test Passed");
    println!("   Cardinality: {:.4}", stats.cardinality_ratio);
    println!("   Most Common Freq: {:.2}%", stats.most_common_freqs.get(0).unwrap_or(&0.0) * 100.0);
    println!("   WHERE Clause: {:?}", rec.where_clause);
    println!("   DDL: {}", rec.ddl);
}

#[tokio::test]
#[ignore]
async fn test_hash_index_for_high_cardinality() {
    let pool = setup_test_db().await;
    create_test_tables(&pool).await;
    
    let mut collector = ColumnStatsCollector::new(pool.clone());
    let stats = collector
        .collect_column_stats("public", "test_sessions", "session_id")
        .await
        .expect("Failed to collect stats");
    
    // Verify high cardinality
    assert!(stats.cardinality_ratio > 0.9, "Expected high cardinality for UUIDs");
    
    // Test index type selection
    let selector = IndexTypeSelector::new();
    let rec = selector.select_index_type(&stats, &[]);
    
    // Should recommend HASH for high cardinality equality queries
    assert!(
        rec.index_type == pg_query_advisor::analyzer::advanced::index_type_selector::IndexType::Hash ||
        rec.index_type == pg_query_advisor::analyzer::advanced::index_type_selector::IndexType::BTree,
        "Expected HASH or BTREE for high cardinality"
    );
    
    println!("✅ High Cardinality Test Passed");
    println!("   Cardinality: {:.4}", stats.cardinality_ratio);
    println!("   Index Type: {:?}", rec.index_type);
    println!("   DDL: {}", rec.ddl);
}

#[tokio::test]
#[ignore]
async fn test_advanced_analyzer_end_to_end() {
    let pool = setup_test_db().await;
    create_test_tables(&pool).await;
    
    let mut analyzer = AdvancedIndexAnalyzer::new(pool.clone());
    
    // Analyze test_events table
    let recommendation = analyzer
        .analyze_table("public", "test_events")
        .await
        .expect("Failed to analyze table");
    
    assert!(
        !recommendation.single_column_indexes.is_empty(),
        "Expected at least one index recommendation"
    );
    
    assert!(
        recommendation.estimated_total_improvement > 0.0,
        "Expected positive improvement estimate"
    );
    
    println!("✅ End-to-End Test Passed");
    println!("   Priority: {}", recommendation.priority);
    println!("   Impact: {}", recommendation.impact.as_str());
    println!("   Total Improvement: {:.1}%", recommendation.estimated_total_improvement);
    println!("   Total Size: {:.2}MB", recommendation.estimated_total_size_mb);
    println!("   Recommendations: {}", recommendation.single_column_indexes.len());
    
    for (i, idx_rec) in recommendation.single_column_indexes.iter().enumerate() {
        println!("\n   {}. {} index on {}", 
            i + 1, 
            idx_rec.index_type.as_str(),
            idx_rec.columns.join(", ")
        );
        println!("      Confidence: {:.0}%", idx_rec.confidence_score * 100.0);
        println!("      Size: {:.2}MB", idx_rec.estimated_size_mb);
        println!("      Improvement: {:.1}%", idx_rec.estimated_improvement_pct);
    }
}

#[tokio::test]
#[ignore]
async fn test_cache_performance() {
    let pool = setup_test_db().await;
    create_test_tables(&pool).await;
    
    let mut collector = ColumnStatsCollector::new(pool.clone());
    
    // First call - should hit database
    let start = std::time::Instant::now();
    let stats1 = collector
        .collect_column_stats("public", "test_events", "created_at")
        .await
        .expect("Failed to collect stats");
    let first_duration = start.elapsed();
    
    // Second call - should hit cache
    let start = std::time::Instant::now();
    let stats2 = collector
        .collect_column_stats("public", "test_events", "created_at")
        .await
        .expect("Failed to collect stats");
    let cached_duration = start.elapsed();
    
    assert_eq!(stats1.column_name, stats2.column_name);
    assert!(
        cached_duration < first_duration / 2,
        "Cache should be significantly faster"
    );
    
    println!("✅ Cache Performance Test Passed");
    println!("   First call: {:?}", first_duration);
    println!("   Cached call: {:?}", cached_duration);
    println!("   Speedup: {:.1}x", first_duration.as_secs_f64() / cached_duration.as_secs_f64());
}

#[tokio::test]
#[ignore]
async fn test_conversion_to_recommendations() {
    let pool = setup_test_db().await;
    create_test_tables(&pool).await;
    
    let mut analyzer = AdvancedIndexAnalyzer::new(pool.clone());
    let advanced_rec = analyzer
        .analyze_table("public", "test_events")
        .await
        .expect("Failed to analyze table");
    
    // Convert to standard recommendations
    let recommendations = pg_query_advisor::analyzer::advanced::convert_to_recommendations(advanced_rec);
    
    assert!(!recommendations.is_empty(), "Expected at least one recommendation");
    
    for rec in &recommendations {
        // Verify required fields
        assert!(!rec.id.is_empty(), "ID should not be empty");
        assert!(!rec.title.is_empty(), "Title should not be empty");
        assert!(!rec.description.is_empty(), "Description should not be empty");
        assert!(!rec.migration_sql.is_empty(), "Migration SQL should not be empty");
        assert!(!rec.safety_notes.is_empty(), "Safety notes should not be empty");
        
        // Verify SQL contains required keywords
        assert!(
            rec.migration_sql.contains("CREATE INDEX CONCURRENTLY"),
            "Should use CONCURRENTLY"
        );
        assert!(
            rec.migration_sql.contains("USING"),
            "Should specify index type"
        );
    }
    
    println!("✅ Recommendation Conversion Test Passed");
    println!("   Converted {} recommendations", recommendations.len());
    
    for (i, rec) in recommendations.iter().enumerate() {
        println!("\n   {}. [{}] {}", i + 1, rec.severity.as_str(), rec.title);
        println!("      Estimated Savings: ${:.0}/mo", 
            rec.estimated_cost_savings_usd_monthly.unwrap_or(0.0));
    }
}

// Cleanup helper
async fn cleanup_test_tables(pool: &PgPool) {
    let _ = sqlx::query("DROP TABLE IF EXISTS test_events CASCADE")
        .execute(pool)
        .await;
    let _ = sqlx::query("DROP TABLE IF EXISTS test_orders CASCADE")
        .execute(pool)
        .await;
    let _ = sqlx::query("DROP TABLE IF EXISTS test_products CASCADE")
        .execute(pool)
        .await;
    let _ = sqlx::query("DROP TABLE IF EXISTS test_sessions CASCADE")
        .execute(pool)
        .await;
}

#[tokio::test]
#[ignore]
async fn test_cleanup() {
    let pool = setup_test_db().await;
    cleanup_test_tables(&pool).await;
    println!("✅ Test tables cleaned up");
}
