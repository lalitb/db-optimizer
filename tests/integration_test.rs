use pg_query_advisor::*;

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
    let recommendations = engine.generate_recommendations(&queries, &tables, &indexes);
    
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
    
    let analyzer = QueryAnalyzer::new();
    
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
    
    let orchestrator = AutoApplyOrchestrator::new(pool, config);
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
    
    let orchestrator = AutoApplyOrchestrator::new(pool, config);
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
    
    let orchestrator = AutoApplyOrchestrator::new(pool, config);
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
