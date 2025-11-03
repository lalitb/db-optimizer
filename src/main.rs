use clap::Parser;
use colored::*;
use sqlx::postgres::PgPoolOptions;
use std::path::PathBuf;
use anyhow::Result;
use tokio::signal;
use tokio::time::{interval, Duration};

use pg_query_advisor::{
    DatabaseCollector, RecommendationEngine, OutputFormatter, AnalysisReport, Severity,
    AutoApplyOrchestrator, AutoApplyConfig, RiskLevel,
    agent::{AgentState, AnalysisResults},
};

#[derive(Parser)]
#[command(name = "pg-query-advisor")]
#[command(version = "0.1.0")]
#[command(about = "PostgreSQL Query Health Advisor - Find performance issues and cost savings", long_about = None)]
struct Cli {
    /// PostgreSQL connection string (or set DATABASE_URL env var)
    /// Format: postgresql://user:password@host:port/database
    #[arg(short, long, env = "DATABASE_URL")]
    database_url: String,

    /// Output format
    #[arg(short = 'f', long, value_enum, default_value = "terminal")]
    format: OutputFormat,

    /// Output file (defaults to stdout)
    #[arg(short, long)]
    output: Option<PathBuf>,

    /// Number of queries to analyze (top N by total time)
    #[arg(short = 'n', long, default_value = "100")]
    query_limit: i64,

    /// Database compute cost per hour (USD) for cost estimation
    #[arg(short = 'c', long, default_value = "0.07")]
    cost_per_hour: f64,

    /// Minimum severity to include in report
    #[arg(short = 's', long, value_enum, default_value = "low")]
    min_severity: MinSeverity,

    /// Enable verbose logging
    #[arg(short, long)]
    verbose: bool,

    // ===== Agent Mode Arguments =====
    /// Run as background agent (continuous monitoring)
    #[arg(long)]
    agent: bool,

    /// Monitoring interval in minutes (agent mode only)
    #[arg(long, default_value = "15")]
    interval_minutes: u64,

    /// State file to track applied optimizations (agent mode)
    #[arg(long, default_value = "~/.pg-advisor-state.json")]
    state_file: String,

    // ===== Auto-Apply Arguments =====
    /// Enable auto-apply mode to automatically implement recommendations
    #[arg(long)]
    auto_apply: bool,

    /// Dry-run mode: simulate without executing (recommended for first use)
    #[arg(long, default_value = "true")]
    dry_run: bool,

    /// Minimum estimated monthly savings (USD) to auto-approve
    #[arg(long, default_value = "50.0")]
    min_savings: f64,

    /// Maximum risk level to auto-approve (low, medium, high, critical)
    #[arg(long, value_enum, default_value = "low")]
    max_risk: MaxRiskLevel,

    /// Duration to monitor impact after applying changes (seconds)
    #[arg(long, default_value = "300")]
    verification_duration: u64,

    /// Minimum improvement percentage to consider successful
    #[arg(long, default_value = "10.0")]
    min_improvement: f64,

    /// Auto-rollback if improvement below threshold
    #[arg(long, default_value = "true")]
    auto_rollback: bool,

    /// Slack webhook URL for notifications (or set SLACK_WEBHOOK env var)
    #[arg(long, env = "SLACK_WEBHOOK")]
    slack_webhook: Option<String>,
}

#[derive(clap::ValueEnum, Clone, Debug)]
enum OutputFormat {
    Terminal,
    Json,
    Markdown,
}

#[derive(clap::ValueEnum, Clone, Debug, PartialEq)]
enum MinSeverity {
    Low,
    Medium,
    High,
    Critical,
}

#[derive(clap::ValueEnum, Clone, Debug, PartialEq)]
enum MaxRiskLevel {
    Low,
    Medium,
    High,
    Critical,
}

impl From<MaxRiskLevel> for RiskLevel {
    fn from(level: MaxRiskLevel) -> Self {
        match level {
            MaxRiskLevel::Low => RiskLevel::Low,
            MaxRiskLevel::Medium => RiskLevel::Medium,
            MaxRiskLevel::High => RiskLevel::High,
            MaxRiskLevel::Critical => RiskLevel::Critical,
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    
    // Setup logging
    if cli.verbose {
        tracing_subscriber::fmt()
            .with_env_filter("pg_query_advisor=debug")
            .init();
    } else {
        tracing_subscriber::fmt()
            .with_env_filter("pg_query_advisor=info")
            .init();
    }
    
    // Branch between agent mode and one-shot mode
    if cli.agent {
        return run_agent_mode(cli).await;
    }
    
    // One-shot mode (original behavior)
    println!("{}", "PostgreSQL Query Health Advisor".bright_cyan().bold());
    println!("{}\n", "Analyzing your database...".bright_black());
    
    // Connect to database
    print!("Connecting to database... ");
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&cli.database_url)
        .await?;
    println!("{}", "✓".bright_green());
    
    // Check if pg_stat_statements is enabled
    print!("Checking pg_stat_statements extension... ");
    let collector = DatabaseCollector::new(pool.clone());
    if !collector.check_pg_stat_statements().await? {
        eprintln!("\n{}", "ERROR: pg_stat_statements extension is not enabled!".bright_red().bold());
        eprintln!("\nTo enable it, run as superuser:");
        eprintln!("  CREATE EXTENSION pg_stat_statements;");
        eprintln!("\nThen restart your application to collect query statistics.");
        std::process::exit(1);
    }
    println!("{}", "✓".bright_green());
    
    // Collect data
    print!("Collecting query statistics... ");
    let queries = collector.collect_query_stats(cli.query_limit).await?;
    println!("{} (analyzed {} queries)", "✓".bright_green(), queries.len());
    
    print!("Collecting table information... ");
    let tables = collector.collect_table_info().await?;
    println!("{} ({} tables)", "✓".bright_green(), tables.len());
    
    print!("Collecting index information... ");
    let indexes = collector.collect_index_info().await?;
    println!("{} ({} indexes)", "✓".bright_green(), indexes.len());
    
    let (db_name, pg_version) = collector.get_database_info().await?;
    
    // Analyze and generate recommendations
    print!("Analyzing queries and generating recommendations... ");
    let engine = RecommendationEngine::new()
        .with_cost_per_hour(cli.cost_per_hour);
    
    let mut recommendations = engine.generate_recommendations(&queries, &tables, &indexes);
    
    // Filter by minimum severity
    recommendations.retain(|rec| {
        match cli.min_severity {
            MinSeverity::Low => true,
            MinSeverity::Medium => rec.severity >= Severity::Medium,
            MinSeverity::High => rec.severity >= Severity::High,
            MinSeverity::Critical => rec.severity == Severity::Critical,
        }
    });
    
    println!("{}", "✓".bright_green());
    
    // Auto-apply mode
    if cli.auto_apply {
        println!("\n{}", "🤖 AUTO-APPLY MODE".bright_cyan().bold());
        
        if cli.dry_run {
            println!("{}", "   (DRY-RUN: Simulating without executing)".bright_yellow());
        }
        
        println!("{}", "━".repeat(70).bright_blue());
        
        // Build auto-apply configuration
        let config = AutoApplyConfig {
            enabled: true,
            dry_run: cli.dry_run,
            min_savings_threshold: cli.min_savings,
            max_risk_level: cli.max_risk.into(),
            verification_duration_secs: cli.verification_duration,
            min_improvement_threshold: cli.min_improvement,
            auto_rollback_on_failure: cli.auto_rollback,
            execution_timeout_secs: 600,
            require_manual_approval: vec![], // Could be configurable
            slack_webhook: cli.slack_webhook.clone(),
        };
        
        // Create orchestrator and process recommendations
        let orchestrator = AutoApplyOrchestrator::new(pool.clone(), config);
        let apply_results = orchestrator.process_all(&recommendations).await;
        
        // Display results
        display_auto_apply_results(&apply_results);
        
        println!("{}", "━".repeat(70).bright_blue());
    }
    
    // Create report
    let report = AnalysisReport {
        database_name: db_name,
        postgres_version: pg_version,
        analyzed_at: chrono::Utc::now(),
        summary: RecommendationEngine::generate_summary(&recommendations),
        recommendations,
        total_queries_analyzed: queries.len(),
        total_tables: tables.len(),
        total_indexes: indexes.len(),
    };
    
    // Output report
    let output_text = match cli.format {
        OutputFormat::Terminal => OutputFormatter::format_terminal(&report),
        OutputFormat::Json => OutputFormatter::format_json(&report)?,
        OutputFormat::Markdown => OutputFormatter::format_markdown(&report),
    };
    
    if let Some(output_path) = cli.output {
        std::fs::write(&output_path, output_text)?;
        println!("\n{} Report saved to: {}", 
            "✓".bright_green(), 
            output_path.display().to_string().bright_white());
    } else {
        println!("{}", output_text);
    }
    
    // Exit with error code if critical issues found
    if report.summary.critical_issues > 0 {
        std::process::exit(1);
    }
    
    Ok(())
}

fn display_auto_apply_results(results: &[pg_query_advisor::ApplyResult]) {
    use pg_query_advisor::ApplyStatus;
    
    let mut successful = 0;
    let mut rolled_back = 0;
    let mut failed = 0;
    let mut skipped = 0;
    let mut pending = 0;
    let mut _total_savings = 0.0;
    
    for (i, result) in results.iter().enumerate() {
        println!("\n[{}/{}] {}", i + 1, results.len(), result.recommendation_id.bright_white());
        
        match result.status {
            ApplyStatus::Success => {
                successful += 1;
                println!("   {} {}", "✅".bright_green(), "SUCCESS".bright_green().bold());
                if let Some(improvement) = result.actual_improvement_pct {
                    println!("   Improvement: {:.1}%", improvement);
                    _total_savings += result.baseline.table_size_mb.unwrap_or(0.0);
                }
            }
            ApplyStatus::RolledBack => {
                rolled_back += 1;
                println!("   {} {}", "⚠️ ".yellow(), "ROLLED BACK".yellow().bold());
                if let Some(improvement) = result.actual_improvement_pct {
                    println!("   Improvement: {:.1}% (below threshold)", improvement);
                }
                println!("   Change was automatically reversed");
            }
            ApplyStatus::Failed => {
                failed += 1;
                println!("   {} {}", "❌".bright_red(), "FAILED".bright_red().bold());
                if let Some(error) = &result.error_message {
                    println!("   Error: {}", error.bright_red());
                }
            }
            ApplyStatus::Skipped => {
                skipped += 1;
                println!("   {} {}", "⏭️ ".bright_black(), "SKIPPED".bright_black());
                if let Some(reason) = &result.error_message {
                    println!("   Reason: {}", reason);
                }
            }
            ApplyStatus::PendingApproval => {
                pending += 1;
                println!("   {} {}", "🔔".bright_yellow(), "PENDING APPROVAL".bright_yellow().bold());
                println!("   Requires manual approval before execution");
            }
            ApplyStatus::Simulated => {
                println!("   {} {}", "🔍".bright_blue(), "SIMULATED".bright_blue().bold());
                println!("   (Dry-run mode - no changes made)");
            }
            ApplyStatus::InProgress => {
                println!("   {} {}", "⏳".bright_yellow(), "IN PROGRESS".bright_yellow());
            }
        }
        
        println!("   Duration: {:.1}s", result.duration_secs);
    }
    
    // Summary
    println!("\n{}", "SUMMARY".bright_cyan().bold());
    println!("{}", "━".repeat(70).bright_blue());
    
    if successful > 0 {
        println!("  {} Successful: {}", "✅".bright_green(), successful.to_string().bright_green().bold());
    }
    if rolled_back > 0 {
        println!("  {} Rolled back: {}", "⚠️ ".yellow(), rolled_back.to_string().yellow().bold());
    }
    if failed > 0 {
        println!("  {} Failed: {}", "❌".bright_red(), failed.to_string().bright_red().bold());
    }
    if skipped > 0 {
        println!("  {} Skipped: {}", "⏭️ ".bright_black(), skipped.to_string().bright_black());
    }
    if pending > 0 {
        println!("  {} Pending approval: {}", "🔔".bright_yellow(), pending.to_string().bright_yellow().bold());
    }
    
    if successful > 0 {
        println!("\n  {} Your database is now faster!", "🚀".bright_cyan());
    }
}

/// Run agent mode - continuous monitoring
async fn run_agent_mode(cli: Cli) -> Result<()> {
    println!("{}", "🤖 PostgreSQL Query Health Advisor - Agent Mode".bright_cyan().bold());
    println!("{}", "━".repeat(70).bright_blue());
    println!("   Monitoring interval: {} minutes", cli.interval_minutes);
    println!("   Database: {}", cli.database_url.split('@').last().unwrap_or("***"));
    println!("   Auto-apply: {}", if cli.auto_apply { "enabled".bright_green() } else { "disabled".bright_black() });
    println!("   Dry-run: {}", if cli.dry_run { "yes".bright_yellow() } else { "no".bright_white() });
    println!("{}", "━".repeat(70).bright_blue());
    println!();
    
    // Expand tilde in state file path
    let state_file = if cli.state_file.starts_with("~/") {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        cli.state_file.replacen("~", &home, 1)
    } else {
        cli.state_file.clone()
    };
    
    // Load or create state
    let mut state = AgentState::load(&state_file)?;
    println!("📊 Loaded state: {} optimizations applied previously\n", state.total_optimizations);
    
    // Create interval timer
    let mut tick_interval = interval(Duration::from_secs(cli.interval_minutes * 60));
    
    println!("✅ Agent started successfully");
    println!("   Press Ctrl+C to stop\n");
    
    // Main monitoring loop
    loop {
        tokio::select! {
            _ = tick_interval.tick() => {
                println!("{}", "━".repeat(70).bright_blue());
                println!("🔍 Running analysis cycle... ({})", chrono::Utc::now().format("%Y-%m-%d %H:%M:%S"));
                
                match run_analysis_cycle(&cli, &mut state).await {
                    Ok(results) => {
                        display_cycle_results(&results);
                        
                        // Save state
                        if let Err(e) = state.save(&state_file) {
                            eprintln!("⚠️  Failed to save state: {}", e);
                        }
                    }
                    Err(e) => {
                        eprintln!("❌ Analysis cycle failed: {}", e);
                        eprintln!("   Will retry in {} minutes", cli.interval_minutes);
                    }
                }
            }
            
            _ = signal::ctrl_c() => {
                println!("\n\n{}", "🛑 Shutdown signal received".bright_yellow().bold());
                println!("   Saving state and exiting...");
                
                if let Err(e) = state.save(&state_file) {
                    eprintln!("⚠️  Failed to save final state: {}", e);
                }
                
                println!("\n{}", "Agent Statistics:".bright_cyan().bold());
                println!("   Total optimizations applied: {}", state.total_optimizations);
                println!("   Uptime: {:.1} hours", state.uptime_hours());
                println!("\n✅ Agent stopped gracefully");
                
                break;
            }
        }
    }
    
    Ok(())
}

/// Run a single analysis cycle
async fn run_analysis_cycle(cli: &Cli, state: &mut AgentState) -> Result<AnalysisResults> {
    // Connect to database
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&cli.database_url)
        .await?;
    
    // Collect data
    let collector = DatabaseCollector::new(pool.clone());
    let queries = collector.collect_query_stats(cli.query_limit).await?;
    let tables = collector.collect_table_info().await?;
    let indexes = collector.collect_index_info().await?;
    
    // Generate recommendations
    let engine = RecommendationEngine::new()
        .with_cost_per_hour(cli.cost_per_hour);
    
    let mut recommendations = engine.generate_recommendations(&queries, &tables, &indexes);
    
    // Filter by severity
    recommendations.retain(|rec| {
        match cli.min_severity {
            MinSeverity::Low => true,
            MinSeverity::Medium => rec.severity >= Severity::Medium,
            MinSeverity::High => rec.severity >= Severity::High,
            MinSeverity::Critical => rec.severity == Severity::Critical,
        }
    });
    
    // Filter out already-applied recommendations
    recommendations.retain(|rec| !state.is_applied(&rec.id));
    
    let mut results = AnalysisResults {
        new_recommendations: recommendations.len(),
        ..Default::default()
    };
    
    if recommendations.is_empty() {
        state.update_last_run();
        return Ok(results);
    }
    
    // Calculate total potential savings
    results.total_savings_usd = recommendations
        .iter()
        .filter_map(|r| r.estimated_cost_savings_usd_monthly)
        .sum();
    
    // Auto-apply if enabled
    if cli.auto_apply {
        let config = AutoApplyConfig {
            enabled: true,
            dry_run: cli.dry_run,
            min_savings_threshold: cli.min_savings,
            max_risk_level: cli.max_risk.clone().into(),
            verification_duration_secs: cli.verification_duration,
            min_improvement_threshold: cli.min_improvement,
            auto_rollback_on_failure: cli.auto_rollback,
            execution_timeout_secs: 600,
            require_manual_approval: vec![],
            slack_webhook: cli.slack_webhook.clone(),
        };
        
        let orchestrator = AutoApplyOrchestrator::new(pool.clone(), config);
        let apply_results = orchestrator.process_all(&recommendations).await;
        
        // Track results
        for result in &apply_results {
            use pg_query_advisor::ApplyStatus;
            
            match result.status {
                ApplyStatus::Success => {
                    results.applied_count += 1;
                    state.mark_applied(result.recommendation_id.clone());
                }
                ApplyStatus::Failed | ApplyStatus::RolledBack => {
                    results.failed_count += 1;
                }
                ApplyStatus::Skipped | ApplyStatus::PendingApproval => {
                    results.skipped_count += 1;
                }
                _ => {}
            }
        }
    } else {
        results.skipped_count = recommendations.len();
    }
    
    state.update_last_run();
    Ok(results)
}

/// Display results from an analysis cycle
fn display_cycle_results(results: &AnalysisResults) {
    if results.is_empty() {
        println!("   ✅ No new optimization opportunities found");
        return;
    }
    
    println!("   📊 Found {} new optimization opportunities", results.new_recommendations);
    
    if results.applied_count > 0 {
        println!("   ✅ Applied: {}", results.applied_count.to_string().bright_green().bold());
    }
    if results.failed_count > 0 {
        println!("   ❌ Failed: {}", results.failed_count.to_string().bright_red());
    }
    if results.skipped_count > 0 {
        println!("   ⏭️  Skipped: {}", results.skipped_count);
    }
    if results.total_savings_usd > 0.0 {
        println!("   💰 Potential savings: ${:.0}/month", results.total_savings_usd);
    }
}
