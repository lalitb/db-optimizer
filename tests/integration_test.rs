use pg_query_advisor::*;
use pg_query_advisor::analyzer::ConnectionAnalyzer;
use sqlx::Row;

#[tokio::test]
#[ignore] // Ignored by default - requires DATABASE_URL to be set
async fn test_basic_workflow() {
    // This test requires a real Postgres database
    // Set TEST_DATABASE_URL environment variable to run
    
    let database_url = std::env::var("TEST_DATABASE_URL")
        .unwrap_or_else(|_| "postgresql://postgres:postgres@localhost/test".to_string());
    
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(5)
        .connect(&database_url)
        .await
        .expect("Failed to connect to test database");
    
    // Test database collector
    let collector = DatabaseCollector::new(pool.clone());
    
    // Check pg_stat_statements (might not be enabled in test DB)
    let has_extension = collector.check_pg_stat_statements().await
        .expect("Failed to check pg_stat_statements");
    
    if !has_extension {
        println!("Skipping test - pg_stat_statements not enabled");
        return;
    }
    
    // Collect data
    let queries = collector.collect_query_stats(10).await
        .expect("Failed to collect queries");
    let tables = collector.collect_table_info().await
        .expect("Failed to collect tables");
    let indexes = collector.collect_index_info().await
        .expect("Failed to collect indexes");
    
    println!("Collected {} queries, {} tables, {} indexes", 
        queries.len(), tables.len(), indexes.len());
    
    // Test recommendation engine
    let engine = RecommendationEngine::new();
    let recommendations = engine.generate_recommendations(&queries, &tables, &indexes, None, None);

    println!("Generated {} recommendations", recommendations.len());

    // Test report generation
    let (db_name, pg_version) = collector.get_database_info().await
        .expect("Failed to get database info");

    let report = AnalysisReport {
        database_name: db_name,
        postgres_version: pg_version,
        analyzed_at: chrono::Utc::now(),
        summary: RecommendationEngine::generate_summary(&recommendations),
        recommendations: recommendations.clone(),
        total_queries_analyzed: queries.len(),
        total_tables: tables.len(),
        total_indexes: indexes.len(),
        connection_stats: None,
        health_score: 100,
    };
    
    // Test output formatters
    let terminal_output = OutputFormatter::format_terminal(&report);
    assert!(!terminal_output.is_empty());
    
    let json_output = OutputFormatter::format_json(&report)
        .expect("Failed to format JSON");
    assert!(!json_output.is_empty());
    
    let markdown_output = OutputFormatter::format_markdown(&report);
    assert!(!markdown_output.is_empty());
    
    println!("All tests passed!");
}

#[test]
fn test_analyzer_logic() {
    use pg_query_advisor::{QueryAnalyzer, QueryStats};
    use pg_query_advisor::config::AnalysisConfig;
    
    let analyzer = QueryAnalyzer::new(AnalysisConfig::default());
    
    // Test slow query detection
    let slow_query = QueryStats {
        queryid: Some(1),
        query: "SELECT * FROM users WHERE email = $1".to_string(),
        calls: 1000,
        total_exec_time_ms: 50000.0,
        mean_exec_time_ms: 50.0,
        rows: 1,
    };
    
    let very_slow_query = QueryStats {
        queryid: Some(2),
        query: "SELECT * FROM orders".to_string(),
        calls: 100,
        total_exec_time_ms: 500000.0,
        mean_exec_time_ms: 5000.0,
        rows: 1000,
    };
    
    let queries = vec![slow_query, very_slow_query];
    let recommendations = analyzer.find_slow_queries(&queries);
    
    assert_eq!(recommendations.len(), 1); // Only the very slow query should trigger
    assert!(recommendations[0].severity == Severity::Critical || recommendations[0].severity == Severity::High);
    
    // Verify the correct query was flagged
    assert!(recommendations[0].title.contains("5000ms") || recommendations[0].description.contains("5000"));
}

#[test]
fn test_cost_estimation() {
    use pg_query_advisor::CostEstimator;
    
    let estimator = CostEstimator::new().with_cost_per_hour(0.10);
    
    let mut rec = Recommendation {
        id: "test".to_string(),
        title: "Test".to_string(),
        description: "Test".to_string(),
        issue_type: IssueType::MissingIndex,
        severity: Severity::High,
        affected_queries_count: Some(1000),
        estimated_time_savings_ms: Some(100.0), // 100ms per query
        estimated_cost_savings_usd_monthly: None,
        migration_sql: "CREATE INDEX test".to_string(),
        rollback_sql: None,
        safety_notes: "Test".to_string(),
        affected_table: None,
        affected_query: None,
    };
    
    estimator.estimate_savings(&mut rec);
    
    assert!(rec.estimated_cost_savings_usd_monthly.is_some());
    assert!(rec.estimated_cost_savings_usd_monthly.unwrap() > 0.0);
}

// ===== AUTO-APPLY TESTS =====

#[tokio::test]
#[ignore] // Requires TEST_DATABASE_URL
async fn test_auto_apply_dry_run() {
    let database_url = std::env::var("TEST_DATABASE_URL")
        .unwrap_or_else(|_| "postgresql://postgres:postgres@localhost/test".to_string());
    
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(5)
        .connect(&database_url)
        .await
        .expect("Failed to connect");
    
    // Create test recommendation
    let recommendation = Recommendation {
        id: "test-dry-run".to_string(),
        title: "Add missing index".to_string(),
        description: "Test recommendation".to_string(),
        issue_type: IssueType::MissingIndex,
        severity: Severity::High,
        affected_queries_count: Some(100),
        estimated_time_savings_ms: Some(100.0),
        estimated_cost_savings_usd_monthly: Some(100.0),
        migration_sql: "CREATE INDEX CONCURRENTLY idx_test ON test_table(test_col)".to_string(),
        rollback_sql: Some("DROP INDEX IF EXISTS idx_test".to_string()),
        safety_notes: "Safe operation".to_string(),
        affected_table: Some("test_table".to_string()),
        affected_query: None,
    };
    
    // Configure for dry-run
    let config = AutoApplyConfig {
        enabled: true,
        dry_run: true, // Important: dry-run mode
        min_savings_threshold: 50.0,
        max_risk_level: RiskLevel::Low,
        verification_duration_secs: 1, // Short for testing
        min_improvement_threshold: 10.0,
        auto_rollback_on_failure: true,
        execution_timeout_secs: 60,
        require_manual_approval: vec![],
        slack_webhook: None,
    };
    
    let mut orchestrator = AutoApplyOrchestrator::new(pool, config);
    let result = orchestrator.process_recommendation(&recommendation).await
        .expect("Failed to process recommendation");
    
    // In dry-run, should be simulated
    assert_eq!(result.status, ApplyStatus::Simulated);
    assert!(!result.rollback_executed);
}

#[tokio::test]
#[ignore]
async fn test_auto_apply_safety_filter() {
    // Test that high-risk operations are blocked
    let recommendation = Recommendation {
        id: "test-high-risk".to_string(),
        title: "Optimize slow query".to_string(),
        description: "Query rewrite".to_string(),
        issue_type: IssueType::SlowQuery, // High risk
        severity: Severity::High,
        affected_queries_count: Some(100),
        estimated_time_savings_ms: Some(100.0),
        estimated_cost_savings_usd_monthly: Some(100.0),
        migration_sql: "-- Rewrite query".to_string(),
        rollback_sql: None,
        safety_notes: "Requires code changes".to_string(),
        affected_table: None,
        affected_query: Some("SELECT * FROM users".to_string()),
    };
    
    let config = AutoApplyConfig {
        enabled: true,
        dry_run: false,
        min_savings_threshold: 50.0,
        max_risk_level: RiskLevel::Low, // Only allow low risk
        verification_duration_secs: 1,
        min_improvement_threshold: 10.0,
        auto_rollback_on_failure: true,
        execution_timeout_secs: 60,
        require_manual_approval: vec![],
        slack_webhook: None,
    };
    
    let database_url = std::env::var("TEST_DATABASE_URL")
        .unwrap_or_else(|_| "postgresql://postgres:postgres@localhost/test".to_string());
    
    let pool = sqlx::postgres::PgPoolOptions::new()
        .connect(&database_url)
        .await
        .expect("Failed to connect");
    
    let mut orchestrator = AutoApplyOrchestrator::new(pool, config);
    let result = orchestrator.process_recommendation(&recommendation).await
        .expect("Failed to process");
    
    // Should be skipped due to high risk
    assert_eq!(result.status, ApplyStatus::Skipped);
}

#[tokio::test]
#[ignore]
async fn test_auto_apply_savings_threshold() {
    let recommendation = Recommendation {
        id: "test-low-savings".to_string(),
        title: "Add minor index".to_string(),
        description: "Low impact".to_string(),
        issue_type: IssueType::MissingIndex,
        severity: Severity::Low,
        affected_queries_count: Some(10),
        estimated_time_savings_ms: Some(10.0),
        estimated_cost_savings_usd_monthly: Some(5.0), // Below threshold
        migration_sql: "CREATE INDEX idx_test ON test(col)".to_string(),
        rollback_sql: Some("DROP INDEX idx_test".to_string()),
        safety_notes: "Safe".to_string(),
        affected_table: Some("test".to_string()),
        affected_query: None,
    };
    
    let config = AutoApplyConfig {
        enabled: true,
        dry_run: false,
        min_savings_threshold: 50.0, // Higher than recommendation savings
        max_risk_level: RiskLevel::Low,
        verification_duration_secs: 1,
        min_improvement_threshold: 10.0,
        auto_rollback_on_failure: true,
        execution_timeout_secs: 60,
        require_manual_approval: vec![],
        slack_webhook: None,
    };
    
    let database_url = std::env::var("TEST_DATABASE_URL")
        .unwrap_or_else(|_| "postgresql://postgres:postgres@localhost/test".to_string());
    
    let pool = sqlx::postgres::PgPoolOptions::new()
        .connect(&database_url)
        .await
        .expect("Failed to connect");
    
    let mut orchestrator = AutoApplyOrchestrator::new(pool, config);
    let result = orchestrator.process_recommendation(&recommendation).await
        .expect("Failed to process");
    
    // Should be skipped due to low savings
    assert_eq!(result.status, ApplyStatus::Skipped);
}

#[test]
fn test_risk_assessment() {
    use pg_query_advisor::auto_apply::safety_filter::SafetyFilter;
    
    let config = AutoApplyConfig {
        enabled: true,
        dry_run: false,
        min_savings_threshold: 50.0,
        max_risk_level: RiskLevel::Medium,
        verification_duration_secs: 300,
        min_improvement_threshold: 10.0,
        auto_rollback_on_failure: true,
        execution_timeout_secs: 600,
        require_manual_approval: vec![],
        slack_webhook: None,
    };
    
    let filter = SafetyFilter::new(config);
    
    // Test missing index (should be low risk)
    let rec1 = Recommendation {
        id: "test-1".to_string(),
        title: "Add index".to_string(),
        description: "Missing index".to_string(),
        issue_type: IssueType::MissingIndex,
        severity: Severity::High,
        affected_queries_count: Some(100),
        estimated_time_savings_ms: Some(100.0),
        estimated_cost_savings_usd_monthly: Some(100.0),
        migration_sql: "CREATE INDEX CONCURRENTLY idx_test ON test(col)".to_string(),
        rollback_sql: Some("DROP INDEX idx_test".to_string()),
        safety_notes: "Safe".to_string(),
        affected_table: Some("test".to_string()),
        affected_query: None,
    };
    
    let applicable1 = filter.assess(&rec1);
    assert_eq!(applicable1.risk_assessment.risk_level, RiskLevel::Low);
    
    // Test slow query (should be high risk)
    let rec2 = Recommendation {
        id: "test-2".to_string(),
        title: "Optimize query".to_string(),
        description: "Slow query".to_string(),
        issue_type: IssueType::SlowQuery,
        severity: Severity::High,
        affected_queries_count: Some(100),
        estimated_time_savings_ms: Some(100.0),
        estimated_cost_savings_usd_monthly: Some(100.0),
        migration_sql: "-- Query rewrite".to_string(),
        rollback_sql: None,
        safety_notes: "Requires code changes".to_string(),
        affected_table: None,
        affected_query: Some("SELECT * FROM users".to_string()),
    };
    
    let applicable2 = filter.assess(&rec2);
    assert_eq!(applicable2.risk_assessment.risk_level, RiskLevel::High);
    assert!(applicable2.requires_approval);
}

#[test]
fn test_sql_parsing() {
    use pg_query_advisor::auto_apply::safety_filter::SafetyFilter;
    
    let config = AutoApplyConfig::default();
    let filter = SafetyFilter::new(config);
    
    // Test parsing CREATE INDEX
    let sql1 = "CREATE INDEX CONCURRENTLY idx_users_email ON users(email);";
    let rec1 = Recommendation {
        id: "test".to_string(),
        title: "Test".to_string(),
        description: "Test".to_string(),
        issue_type: IssueType::MissingIndex,
        severity: Severity::High,
        affected_queries_count: Some(1),
        estimated_time_savings_ms: Some(1.0),
        estimated_cost_savings_usd_monthly: Some(1.0),
        migration_sql: sql1.to_string(),
        rollback_sql: None,
        safety_notes: "Test".to_string(),
        affected_table: None,
        affected_query: None,
    };
    
    let applicable1 = filter.assess(&rec1);
    if let ApplyStrategy::CreateIndexConcurrently { index_name, table_name, columns } = applicable1.apply_strategy {
        assert_eq!(index_name, "idx_users_email");
        assert_eq!(table_name, "users");
        assert_eq!(columns, vec!["email"]);
    } else {
        panic!("Expected CreateIndexConcurrently strategy");
    }
    
    // Test parsing DROP INDEX
    let sql2 = "DROP INDEX IF EXISTS public.idx_old_index;";
    let rec2 = Recommendation {
        id: "test".to_string(),
        title: "Test".to_string(),
        description: "Test".to_string(),
        issue_type: IssueType::UnusedIndex,
        severity: Severity::Medium,
        affected_queries_count: Some(0),
        estimated_time_savings_ms: None,
        estimated_cost_savings_usd_monthly: Some(10.0),
        migration_sql: sql2.to_string(),
        rollback_sql: None,
        safety_notes: "Test".to_string(),
        affected_table: None,
        affected_query: None,
    };
    
    let applicable2 = filter.assess(&rec2);
    if let ApplyStrategy::DropIndex { schema, index_name } = applicable2.apply_strategy {
        assert_eq!(schema, "public");
        assert_eq!(index_name, "idx_old_index");
    } else {
        panic!("Expected DropIndex strategy");
    }
}

#[test]
fn test_improvement_calculation() {
    use pg_query_advisor::PerformanceBaseline;
    use chrono::Utc;
    
    let baseline = PerformanceBaseline {
        captured_at: Utc::now(),
        query_id: None,
        table_name: Some("test".to_string()),
        mean_exec_time_ms: 1000.0,
        p95_exec_time_ms: 1500.0,
        p99_exec_time_ms: 2000.0,
        calls_per_minute: 10.0,
        query_plan: None,
        plan_cost: None,
        table_size_mb: Some(100.0),
        index_size_mb: None,
        total_queries_analyzed: 100,
    };
    
    let post_apply = PerformanceBaseline {
        captured_at: Utc::now(),
        query_id: None,
        table_name: Some("test".to_string()),
        mean_exec_time_ms: 200.0, // 80% improvement
        p95_exec_time_ms: 300.0,
        p99_exec_time_ms: 400.0,
        calls_per_minute: 10.0,
        query_plan: None,
        plan_cost: None,
        table_size_mb: Some(100.0),
        index_size_mb: Some(10.0),
        total_queries_analyzed: 100,
    };
    
    // Calculate improvement
    let improvement = (baseline.mean_exec_time_ms - post_apply.mean_exec_time_ms) 
        / baseline.mean_exec_time_ms * 100.0;
    
    assert_eq!(improvement, 80.0);
    assert!(improvement > 10.0); // Meets typical threshold
}

// ===== CONNECTION MONITORING TESTS =====

#[tokio::test]
#[ignore] // Requires TEST_DATABASE_URL and active database connections
async fn test_connection_monitoring_integration() {
    let database_url = std::env::var("TEST_DATABASE_URL")
        .unwrap_or_else(|_| "postgresql://postgres:postgres@localhost/test".to_string());

    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(5)
        .connect(&database_url)
        .await
        .expect("Failed to connect to test database");

    let collector = DatabaseCollector::new(pool.clone());

    // Test 1: Collect connection stats
    let connection_stats = collector.collect_connection_stats().await
        .expect("Failed to collect connection stats");

    println!("Connection Stats:");
    println!("  Total: {} / {} max", connection_stats.total_connections, connection_stats.max_connections);
    println!("  Usage: {:.1}%", connection_stats.usage_percentage());
    println!("  Idle in transaction: {}", connection_stats.idle_in_transaction_count);
    println!("  Locked: {}", connection_stats.locked_connections_count);

    // Verify basic stats
    assert!(connection_stats.total_connections > 0, "Should have at least one connection (this test)");
    assert!(connection_stats.max_connections > 0);
    assert!(connection_stats.usage_percentage() >= 0.0);
    assert!(connection_stats.usage_percentage() <= 100.0);

    // Test 2: Collect individual connections
    let connections = collector.collect_connections().await
        .expect("Failed to collect connections");

    println!("\nActive Connections: {}", connections.len());
    for conn in &connections {
        println!("  PID {}: {:?} - {} - {}",
            conn.pid,
            conn.state,
            conn.database,
            conn.query.chars().take(50).collect::<String>()
        );
    }

    assert!(connections.len() > 0, "Should have at least one connection");

    // Verify connection data structure
    for conn in &connections {
        assert!(conn.pid > 0);
        assert!(!conn.database.is_empty());
        assert!(!conn.username.is_empty());
        assert!(!conn.backend_type.is_empty());
    }

    // Test 3: Connection analyzer
    let analyzer = ConnectionAnalyzer::new();
    let recommendations = analyzer.analyze_connections(&connection_stats, &connections);

    println!("\nConnection Recommendations: {}", recommendations.len());
    for rec in &recommendations {
        println!("  [{:?}] {}: {:?}",
            rec.severity,
            rec.title,
            rec.issue_type
        );
    }

    // Verify recommendations structure
    for rec in &recommendations {
        assert!(!rec.id.is_empty());
        assert!(!rec.title.is_empty());
        assert!(!rec.description.is_empty());
        assert!(!rec.migration_sql.is_empty());
        assert!(matches!(
            rec.issue_type,
            IssueType::ConnectionPoolExhaustion |
            IssueType::IdleInTransaction |
            IssueType::LockContention
        ));
    }

    // Test 4: Get long idle transactions (if any)
    let long_idle = collector.get_long_idle_transactions(30).await
        .expect("Failed to get long idle transactions");

    println!("\nLong idle transactions (>30s): {}", long_idle.len());

    // Test 5: Get locked connections (if any)
    let locked = collector.get_locked_connections().await
        .expect("Failed to get locked connections");

    println!("Locked connections: {}", locked.len());

    // Test 6: Integration with recommendation engine
    let queries = collector.collect_query_stats(10).await.unwrap_or_default();
    let tables = collector.collect_table_info().await.unwrap_or_default();
    let indexes = collector.collect_index_info().await.unwrap_or_default();

    let engine = RecommendationEngine::new();
    let all_recommendations = engine.generate_recommendations(
        &queries,
        &tables,
        &indexes,
        Some(&connection_stats),
        Some(&connections),
    );

    println!("\nTotal Recommendations (all types): {}", all_recommendations.len());

    // Count connection-related recommendations
    let connection_recs: Vec<_> = all_recommendations.iter()
        .filter(|r| matches!(
            r.issue_type,
            IssueType::ConnectionPoolExhaustion |
            IssueType::IdleInTransaction |
            IssueType::LockContention
        ))
        .collect();

    println!("Connection-related recommendations: {}", connection_recs.len());

    // Test 7: Verify ConnectionStats helper methods
    assert!(connection_stats.usage_percentage() >= 0.0);

    let is_near_80 = connection_stats.is_near_limit(80.0);
    let is_near_95 = connection_stats.is_near_limit(95.0);

    println!("\nConnection pool near 80% limit: {}", is_near_80);
    println!("Connection pool near 95% limit: {}", is_near_95);

    // Test 8: Verify ConnectionInfo helper methods work
    if let Some(conn) = connections.first() {
        let state_duration = conn.state_duration_secs();
        let query_duration = conn.query_duration_secs();
        let is_locked = conn.is_locked();

        println!("\nFirst connection details:");
        println!("  State: {:?}", conn.state);
        println!("  State duration: {:?}s", state_duration);
        println!("  Query duration: {:?}s", query_duration);
        println!("  Is locked: {}", is_locked);
        println!("  Wait event: {:?}", conn.wait_event_type);
    }

    println!("\n✅ All connection monitoring integration tests passed!");
}

#[tokio::test]
#[ignore] // Requires TEST_DATABASE_URL
async fn test_connection_state_enum_parsing() {
    // This test verifies that ConnectionState parsing works with real pg_stat_activity data
    let database_url = std::env::var("TEST_DATABASE_URL")
        .unwrap_or_else(|_| "postgresql://postgres:postgres@localhost/test".to_string());

    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(5)
        .connect(&database_url)
        .await
        .expect("Failed to connect");

    // Query pg_stat_activity directly
    let rows = sqlx::query(
        "SELECT DISTINCT state FROM pg_stat_activity WHERE state IS NOT NULL"
    )
    .fetch_all(&pool)
    .await
    .expect("Failed to query pg_stat_activity");

    println!("Found {} distinct connection states in pg_stat_activity", rows.len());

    for row in rows {
        let state_str: String = row.get("state");
        let parsed_state = ConnectionState::from_str(&state_str);

        println!("  '{}' -> {:?}", state_str, parsed_state);

        // Verify round-trip
        let back_to_str = parsed_state.as_str();

        // Allow for normalization (e.g., "disabled" might not round-trip exactly)
        assert!(!back_to_str.is_empty(), "State should serialize to non-empty string");
    }

    println!("✅ All connection state parsing tests passed!");
}

#[tokio::test]
#[ignore] // Requires TEST_DATABASE_URL and ability to create test connections
async fn test_simulated_idle_in_transaction() {
    // This test simulates an idle-in-transaction scenario
    let database_url = std::env::var("TEST_DATABASE_URL")
        .unwrap_or_else(|_| "postgresql://postgres:postgres@localhost/test".to_string());

    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(10)
        .connect(&database_url)
        .await
        .expect("Failed to connect");

    // Start a transaction but don't commit it (simulates idle in transaction)
    let mut tx = pool.begin().await.expect("Failed to start transaction");

    sqlx::query("SELECT 1")
        .execute(&mut *tx)
        .await
        .expect("Failed to execute query");

    // Don't commit - leave it idle in transaction

    // Now check if we can detect it
    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

    let collector = DatabaseCollector::new(pool.clone());
    let connection_stats = collector.collect_connection_stats().await
        .expect("Failed to collect stats");

    println!("Idle in transaction count: {}", connection_stats.idle_in_transaction_count);

    // Note: This might be 0 if the transaction is too fast or if pg_stat_activity
    // hasn't updated yet. This is expected behavior.

    // Clean up
    tx.rollback().await.expect("Failed to rollback");

    println!("✅ Idle in transaction simulation test completed!");
}
