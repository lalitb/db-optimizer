use clap::{Parser, Subcommand};
use colored::*;
use sqlx::postgres::PgPoolOptions;
use sqlx::Row;
use std::path::PathBuf;
use anyhow::Result;
use tokio::signal;
use tokio::time::{interval, Duration};

use pg_query_advisor::{
    DatabaseCollector, RecommendationEngine, OutputFormatter, AnalysisReport, Severity,
    AutoApplyOrchestrator, AutoApplyConfig, RiskLevel,
    agent::{AgentState, AnalysisResults},
    query_rewriter::cli::{RewriterCli, InputSource, OutputFormat as RewriterOutputFormat},
};

#[derive(Parser)]
#[command(name = "pg-query-advisor")]
#[command(version = "0.1.0")]
#[command(about = "PostgreSQL Query Health Advisor - Find performance issues and cost savings", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    #[command(flatten)]
    analyze_opts: Option<AnalyzeOptions>,
}

#[derive(Subcommand)]
enum Commands {
    /// Analyze database queries and generate recommendations (default)
    Analyze(AnalyzeOptions),
    
    /// Run as background agent with continuous monitoring
    Agent(AgentOptions),
    
    /// Rewrite and optimize SQL queries
    RewriteSql(RewriteSqlOptions),
    
    /// Start HTTP API server
    Serve(ServeOptions),
    
    /// Tune PostgreSQL configuration (generate optimal settings)
    ConfigTune(ConfigTuneOptions),
    
    /// Validate current PostgreSQL configuration
    ConfigValidate(ConfigValidateOptions),
}

#[derive(Parser)]
struct AnalyzeOptions {
    /// PostgreSQL connection string (or set DATABASE_URL env var)
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

#[derive(Parser)]
struct AgentOptions {
    /// PostgreSQL connection string (or set DATABASE_URL env var)
    #[arg(short, long, env = "DATABASE_URL")]
    database_url: String,

    /// Output format
    #[arg(short = 'f', long, value_enum, default_value = "terminal")]
    format: OutputFormat,

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

    /// Monitoring interval in minutes
    #[arg(long, default_value = "15")]
    interval_minutes: u64,

    /// State file to track applied optimizations
    #[arg(long, default_value = "~/.pg-advisor-state.json")]
    state_file: String,

    /// Enable auto-apply mode
    #[arg(long)]
    auto_apply: bool,

    /// Dry-run mode: simulate without executing
    #[arg(long, default_value = "true")]
    dry_run: bool,

    /// Minimum estimated monthly savings (USD) to auto-approve
    #[arg(long, default_value = "50.0")]
    min_savings: f64,

    /// Maximum risk level to auto-approve
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

    /// Slack webhook URL for notifications
    #[arg(long, env = "SLACK_WEBHOOK")]
    slack_webhook: Option<String>,
}

#[derive(Parser)]
struct ServeOptions {
    /// Port to listen on
    #[arg(short, long, default_value = "8080")]
    port: u16,
    
    /// Enable verbose logging
    #[arg(short, long)]
    verbose: bool,
}

#[derive(Parser)]
struct ConfigTuneOptions {
    /// PostgreSQL connection string (optional, for workload detection)
    #[arg(short, long, env = "DATABASE_URL")]
    database_url: Option<String>,
    
    /// Workload type (auto-detect if not specified and database provided)
    #[arg(short, long, value_enum)]
    workload: Option<WorkloadTypeArg>,
    
    /// System RAM in GB (auto-detect if not specified)
    #[arg(long)]
    ram_gb: Option<f64>,
    
    /// CPU count (auto-detect if not specified)
    #[arg(long)]
    cpus: Option<u32>,
    
    /// Disk type
    #[arg(long, value_enum)]
    disk_type: Option<DiskTypeArg>,
    
    /// Output format
    #[arg(short = 'f', long, value_enum, default_value = "terminal")]
    format: OutputFormat,
    
    /// Output file (defaults to stdout)
    #[arg(short, long)]
    output: Option<PathBuf>,
    
    /// Show before/after comparison (requires database connection)
    #[arg(long)]
    show_current: bool,
}

#[derive(Parser)]
struct ConfigValidateOptions {
    /// PostgreSQL connection string
    #[arg(short, long, env = "DATABASE_URL")]
    database_url: String,
    
    /// Output format
    #[arg(short = 'f', long, value_enum, default_value = "terminal")]
    format: OutputFormat,
    
    /// Output file (defaults to stdout)
    #[arg(short, long)]
    output: Option<PathBuf>,
}

#[derive(clap::ValueEnum, Clone, Debug)]
enum WorkloadTypeArg {
    Desktop,
    Web,
    Oltp,
    DataWarehouse,
    Mixed,
}

#[derive(clap::ValueEnum, Clone, Debug)]
enum DiskTypeArg {
    Hdd,
    Ssd,
    Nvme,
}

#[derive(Parser)]
struct RewriteSqlOptions {
    /// Input SQL file (or use stdin if not provided)
    #[arg(short, long)]
    input: Option<PathBuf>,

    /// SQL query string (alternative to file/stdin)
    #[arg(short = 'q', long)]
    query: Option<String>,

    /// Output format
    #[arg(short = 'f', long, value_enum, default_value = "terminal")]
    format: RewriterFormat,

    /// Include detailed explanations
    #[arg(short = 'e', long, default_value = "true")]
    explain: bool,

    /// Output file (defaults to stdout)
    #[arg(short, long)]
    output: Option<PathBuf>,
}

#[derive(clap::ValueEnum, Clone, Debug)]
enum RewriterFormat {
    Terminal,
    Json,
    Markdown,
}

impl From<RewriterFormat> for RewriterOutputFormat {
    fn from(format: RewriterFormat) -> Self {
        match format {
            RewriterFormat::Terminal => RewriterOutputFormat::Terminal,
            RewriterFormat::Json => RewriterOutputFormat::Json,
            RewriterFormat::Markdown => RewriterOutputFormat::Markdown,
        }
    }
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
    
    // Handle subcommands or default to analyze
    match cli.command {
        Some(Commands::Analyze(opts)) => {
            run_analyze_mode(opts).await
        }
        Some(Commands::Agent(opts)) => {
            run_agent_mode(opts).await
        }
        Some(Commands::RewriteSql(opts)) => {
            run_rewrite_sql_mode(opts)
        }
        Some(Commands::Serve(opts)) => {
            run_serve_mode(opts).await
        }
        Some(Commands::ConfigTune(opts)) => {
            run_config_tune_mode(opts).await
        }
        Some(Commands::ConfigValidate(opts)) => {
            run_config_validate_mode(opts).await
        }
        None => {
            // Default behavior - run analyze with flattened options
            if let Some(opts) = cli.analyze_opts {
                run_analyze_mode(opts).await
            } else {
                eprintln!("Error: No command specified. Use --help for usage information.");
                std::process::exit(1)
            }
        }
    }
}

/// Run rewrite-sql mode
fn run_rewrite_sql_mode(opts: RewriteSqlOptions) -> Result<()> {
    // Determine input source
    let input_source = if let Some(query) = opts.query {
        InputSource::Inline(query)
    } else if let Some(path) = opts.input {
        InputSource::File(path.to_string_lossy().to_string())
    } else {
        InputSource::Stdin
    };
    
    // Create CLI and read input
    let cli = RewriterCli::new();
    let sql = cli.read_input(&input_source)?;
    
    // Rewrite and format
    let output = cli.rewrite_and_format(&sql, opts.format.into(), opts.explain)?;
    
    // Write output
    if let Some(output_path) = opts.output {
        std::fs::write(&output_path, &output)?;
        println!("{} Report saved to: {}", 
            "✓".bright_green(), 
            output_path.display().to_string().bright_white());
    } else {
        print!("{}", output);
    }
    
    Ok(())
}

/// Run analyze mode (one-shot analysis)
async fn run_analyze_mode(cli: AnalyzeOptions) -> Result<()> {
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

    // Collect connection statistics
    print!("Collecting connection statistics... ");
    let connection_stats = collector.collect_connection_stats().await.ok();
    let connections = collector.collect_connections().await.ok();
    println!("{}", "✓".bright_green());

    // Analyze and generate recommendations
    print!("Analyzing queries and generating recommendations... ");
    let engine = RecommendationEngine::new()
        .with_cost_per_hour(cli.cost_per_hour);

    let mut recommendations = engine.generate_recommendations(
        &queries,
        &tables,
        &indexes,
        connection_stats.as_ref(),
        connections.as_deref(),
    );
    
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
        connection_stats,
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
async fn run_agent_mode(opts: AgentOptions) -> Result<()> {
    // Setup logging
    if opts.verbose {
        tracing_subscriber::fmt()
            .with_env_filter("pg_query_advisor=debug")
            .init();
    } else {
        tracing_subscriber::fmt()
            .with_env_filter("pg_query_advisor=info")
            .init();
    }
    println!("{}", "🤖 PostgreSQL Query Health Advisor - Agent Mode".bright_cyan().bold());
    println!("{}", "━".repeat(70).bright_blue());
    println!("   Monitoring interval: {} minutes", opts.interval_minutes);
    println!("   Database: {}", opts.database_url.split('@').last().unwrap_or("***"));
    println!("   Auto-apply: {}", if opts.auto_apply { "enabled".bright_green() } else { "disabled".bright_black() });
    println!("   Dry-run: {}", if opts.dry_run { "yes".bright_yellow() } else { "no".bright_white() });
    println!("{}", "━".repeat(70).bright_blue());
    println!();
    
    // Expand tilde in state file path
    let state_file = if opts.state_file.starts_with("~/") {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        opts.state_file.replacen("~", &home, 1)
    } else {
        opts.state_file.clone()
    };
    
    // Load or create state
    let mut state = AgentState::load(&state_file)?;
    println!("📊 Loaded state: {} optimizations applied previously\n", state.total_optimizations);
    
    // Create interval timer
    let mut tick_interval = interval(Duration::from_secs(opts.interval_minutes * 60));
    
    println!("✅ Agent started successfully");
    println!("   Press Ctrl+C to stop\n");
    
    // Main monitoring loop
    loop {
        tokio::select! {
            _ = tick_interval.tick() => {
                println!("{}", "━".repeat(70).bright_blue());
                println!("🔍 Running analysis cycle... ({})", chrono::Utc::now().format("%Y-%m-%d %H:%M:%S"));
                
                match run_analysis_cycle(&opts, &mut state).await {
                    Ok(results) => {
                        display_cycle_results(&results);
                        
                        // Save state
                        if let Err(e) = state.save(&state_file) {
                            eprintln!("⚠️  Failed to save state: {}", e);
                        }
                    }
                    Err(e) => {
                        eprintln!("❌ Analysis cycle failed: {}", e);
                        eprintln!("   Will retry in {} minutes", opts.interval_minutes);
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
async fn run_analysis_cycle(opts: &AgentOptions, state: &mut AgentState) -> Result<AnalysisResults> {
    // Connect to database
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&opts.database_url)
        .await?;
    
    // Collect data
    let collector = DatabaseCollector::new(pool.clone());
    let queries = collector.collect_query_stats(opts.query_limit).await?;
    let tables = collector.collect_table_info().await?;
    let indexes = collector.collect_index_info().await?;
    let connection_stats = collector.collect_connection_stats().await.ok();
    let connections = collector.collect_connections().await.ok();

    // Generate recommendations
    let engine = RecommendationEngine::new()
        .with_cost_per_hour(opts.cost_per_hour);

    let mut recommendations = engine.generate_recommendations(
        &queries,
        &tables,
        &indexes,
        connection_stats.as_ref(),
        connections.as_deref(),
    );
    
    // Filter by severity
    recommendations.retain(|rec| {
        match opts.min_severity {
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
    if opts.auto_apply {
        let config = AutoApplyConfig {
            enabled: true,
            dry_run: opts.dry_run,
            min_savings_threshold: opts.min_savings,
            max_risk_level: opts.max_risk.clone().into(),
            verification_duration_secs: opts.verification_duration,
            min_improvement_threshold: opts.min_improvement,
            auto_rollback_on_failure: opts.auto_rollback,
            execution_timeout_secs: 600,
            require_manual_approval: vec![],
            slack_webhook: opts.slack_webhook.clone(),
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

/// Run HTTP server mode
async fn run_serve_mode(opts: ServeOptions) -> Result<()> {
    // Setup logging
    if opts.verbose {
        tracing_subscriber::fmt()
            .with_env_filter("pg_query_advisor=debug")
            .init();
    } else {
        tracing_subscriber::fmt()
            .with_env_filter("pg_query_advisor=info")
            .init();
    }
    
    println!("{}", "🌐 PostgreSQL Query Health Advisor - HTTP API Server".bright_cyan().bold());
    println!("{}", "━".repeat(70).bright_blue());
    println!("   Port: {}", opts.port);
    println!("   API Base: {}", format!("http://localhost:{}/api/v1", opts.port).bright_cyan());
    println!();
    println!("   Endpoints:");
    println!("     {}  {}  - Interactive documentation", "GET".bright_green(), "/".bright_white());
    println!("     {}  {}  - Health check", "GET".bright_green(), "/api/v1/health".bright_white());
    println!("     {}  {}  - Lock contention", "GET".bright_green(), "/api/v1/locks/contention".bright_white());
    println!("     {}  {}  - Query rewriter", "POST".bright_yellow(), "/api/v1/queries/rewrite".bright_white());
    println!("     {}  {}  - Prometheus metrics", "GET".bright_green(), "/api/v1/stats/metrics".bright_white());
    println!("     {} See http://localhost:{} for full API docs", "📚".bright_blue(), opts.port);
    println!("{}", "━".repeat(70).bright_blue());
    println!();
    
    // Create minimal HTTP config
    let http_config = pg_query_advisor::HttpServerConfig {
        enabled: true,
        bind_address: "127.0.0.1".to_string(),
        port: opts.port,
        auth_token: None,
        enable_cors: true,
    };
    
    // Create empty persistence (will add proper integration later)
    use std::sync::Arc;
    use tokio::sync::RwLock;
    let persistence: Arc<RwLock<Box<dyn pg_query_advisor::persistence::PersistenceStore>>> = 
        Arc::new(RwLock::new(Box::new(pg_query_advisor::persistence::sqlite::SqliteStore::new(":memory:").await?)));
    
    println!("✅ Server starting...");
    println!("   Access API at: {}", format!("http://localhost:{}/api/v1", opts.port).bright_cyan());
    println!("   View docs at: {}", format!("http://localhost:{}", opts.port).bright_cyan().underline());
    println!("   Press Ctrl+C to stop\n");
    
    // Start new HTTP API server (no eBPF manager, no db_pool for now)
    pg_query_advisor::http::serve(http_config, persistence, None, None).await
}

/// Run config tune mode
async fn run_config_tune_mode(opts: ConfigTuneOptions) -> Result<()> {
    use pg_query_advisor::config_tuner::{ConfigTuner, WorkloadDetector, models::{SystemInfo, WorkloadType, DiskType}};
    
    println!("{}", "⚙️  PostgreSQL Configuration Tuner".bright_cyan().bold());
    println!("{}\n", "Generating optimal configuration...".bright_black());
    
    // Detect or use provided system info
    print!("Detecting system information... ");
    let mut system_info = SystemInfo::detect()?;
    println!("{}", "✓".bright_green());
    
    // Override with user-provided values
    if let Some(ram_gb) = opts.ram_gb {
        system_info.total_ram_kb = (ram_gb * 1024.0 * 1024.0) as u64;
    }
    if let Some(cpus) = opts.cpus {
        system_info.cpu_count = cpus;
    }
    if let Some(disk_type) = opts.disk_type {
        system_info.disk_type = match disk_type {
            DiskTypeArg::Hdd => DiskType::HDD,
            DiskTypeArg::Ssd => DiskType::SSD,
            DiskTypeArg::Nvme => DiskType::SAN, // Use SAN for NVMe-class storage
        };
    }
    
    println!("   RAM: {:.1} GB", system_info.ram_gb());
    println!("   CPUs: {}", system_info.cpu_count);
    println!("   Disk: {:?}", system_info.disk_type);
    println!("   OS: {:?}", system_info.os_type);
    println!("   PostgreSQL: {:.1}", system_info.pg_version);
    println!();
    
    // Determine workload type
    let (workload, detected) = if let Some(workload_arg) = opts.workload {
        let wl = match workload_arg {
            WorkloadTypeArg::Desktop => WorkloadType::Desktop,
            WorkloadTypeArg::Web => WorkloadType::Web,
            WorkloadTypeArg::Oltp => WorkloadType::OLTP,
            WorkloadTypeArg::DataWarehouse => WorkloadType::DataWarehouse,
            WorkloadTypeArg::Mixed => WorkloadType::Mixed,
        };
        (wl, false)
    } else if let Some(db_url) = &opts.database_url {
        print!("Detecting workload type from database... ");
        let pool = PgPoolOptions::new()
            .max_connections(5)
            .connect(db_url)
            .await?;
        
        let detector = WorkloadDetector::new();
        let detected_workload = detector.detect(&pool).await?;
        println!("{}", "✓".bright_green());
        (detected_workload, true)
    } else {
        println!("{}", "⚠️  No workload specified and no database connection, using Mixed".yellow());
        (WorkloadType::Mixed, false)
    };
    
    println!("Workload type: {} {}", 
        format!("{:?}", workload).bright_white().bold(),
        if detected { "(auto-detected)".bright_black() } else { "(manual)".bright_black() }
    );
    println!();
    
    // Show current config if requested
    if opts.show_current {
        if let Some(db_url) = &opts.database_url {
            print!("Fetching current configuration... ");
            let pool = PgPoolOptions::new()
                .max_connections(5)
                .connect(db_url)
                .await?;
            println!("{}", "✓".bright_green());
            
            // Query key params
            let row = sqlx::query("SELECT name, setting, unit FROM pg_settings WHERE name IN ('shared_buffers', 'effective_cache_size', 'work_mem', 'max_connections')")
                .fetch_all(&pool)
                .await?;
            
            println!("\n{}", "Current Configuration:".bright_cyan().bold());
            println!("{}", "━".repeat(50).bright_blue());
            for r in row {
                let name: String = r.try_get("name")?;
                let setting: String = r.try_get("setting")?;
                let unit: Option<String> = r.try_get("unit").ok();
                println!("   {}: {} {}", name, setting, unit.unwrap_or_default());
            }
            println!();
        }
    }
    
    // Calculate optimal configuration
    print!("Calculating optimal parameters... ");
    let tuner = ConfigTuner::new();
    let optimal_config = tuner.calculate_optimal_config(&system_info, workload);
    println!("{}", "✓".bright_green());
    
    // Format output
    let output_text = match opts.format {
        OutputFormat::Terminal => {
            let mut output = String::new();
            output.push_str(&format!("\n{}\n", "Recommended Configuration:".bright_cyan().bold()));
            output.push_str(&format!("{}\n\n", "━".repeat(70).bright_blue()));
            output.push_str(&format!("{}\n", optimal_config.to_postgresql_conf()));
            output.push_str(&format!("\n{}\n", "━".repeat(70).bright_blue()));
            output.push_str(&format!("\n{}\n", "💡 Tips:".bright_yellow().bold()));
            output.push_str("   1. Review these settings before applying\n");
            output.push_str("   2. Test in a non-production environment first\n");
            output.push_str("   3. Restart PostgreSQL after making changes\n");
            output.push_str("   4. Monitor performance after changes\n");
            output
        }
        OutputFormat::Json => {
            serde_json::to_string_pretty(&optimal_config)?
        }
        OutputFormat::Markdown => {
            let mut output = String::new();
            output.push_str("# PostgreSQL Configuration Recommendations\n\n");
            output.push_str(&format!("**Workload Type:** {:?}\n", workload));
            output.push_str(&format!("**System:** {:.1} GB RAM, {} CPUs, {:?} disk\n\n", 
                system_info.ram_gb(), system_info.cpu_count, system_info.disk_type));
            output.push_str("## Recommended Settings\n\n```ini\n");
            output.push_str(&optimal_config.to_postgresql_conf());
            output.push_str("\n```\n");
            output
        }
    };
    
    // Output
    if let Some(output_path) = opts.output {
        std::fs::write(&output_path, &output_text)?;
        println!("\n{} Configuration saved to: {}", 
            "✓".bright_green(), 
            output_path.display().to_string().bright_white());
    } else {
        print!("{}", output_text);
    }
    
    Ok(())
}

/// Run config validate mode
async fn run_config_validate_mode(opts: ConfigValidateOptions) -> Result<()> {
    use pg_query_advisor::config_tuner::ConfigValidator;
    
    println!("{}", "🔍 PostgreSQL Configuration Validator".bright_cyan().bold());
    println!("{}\n", "Analyzing current configuration...".bright_black());
    
    // Connect to database
    print!("Connecting to database... ");
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&opts.database_url)
        .await?;
    println!("{}", "✓".bright_green());
    
    // Run validation
    print!("Running configuration checks... ");
    let validator = ConfigValidator::new();
    let report = validator.validate(&pool).await?;
    println!("{}", "✓".bright_green());
    println!();
    
    // Format output
    let output_text = match opts.format {
        OutputFormat::Terminal => {
            let mut output = String::new();
            
            // Header
            if report.is_healthy() {
                output.push_str(&format!("{} {}\n", "✅".bright_green(), "Configuration looks good!".bright_green().bold()));
            } else {
                output.push_str(&format!("{} {}\n", "⚠️ ".yellow(), "Configuration issues detected".yellow().bold()));
            }
            output.push_str(&format!("{}\n\n", "━".repeat(70).bright_blue()));
            
            // Critical Issues
            let critical_issues: Vec<_> = report.issues.iter()
                .filter(|i| i.severity == pg_query_advisor::config_tuner::models::ValidationSeverity::Critical)
                .collect();
            
            if !critical_issues.is_empty() {
                output.push_str(&format!("{} ({}):\n", "Critical".bright_red().bold(), critical_issues.len()));
                for issue in critical_issues {
                    output.push_str(&format!("   {} [{}] {}: {}\n",
                        "❌".bright_red(),
                        issue.category,
                        issue.title,
                        issue.description
                    ));
                    if !issue.recommendation.is_empty() {
                        output.push_str(&format!("      → {}\n", issue.recommendation));
                    }
                    if let (Some(current), Some(recommended)) = (&issue.current_value, &issue.recommended_value) {
                        output.push_str(&format!("      Current: {}, Recommended: {}\n", current, recommended));
                    }
                }
                output.push_str("\n");
            }
            
            // High Issues
            let high_issues: Vec<_> = report.issues.iter()
                .filter(|i| i.severity == pg_query_advisor::config_tuner::models::ValidationSeverity::High)
                .collect();
            
            if !high_issues.is_empty() {
                output.push_str(&format!("{} ({}):\n", "High".bright_red(), high_issues.len()));
                for issue in high_issues {
                    output.push_str(&format!("   {} [{}] {}: {}\n",
                        "⚠️ ".bright_red(),
                        issue.category,
                        issue.title,
                        issue.description
                    ));
                    if !issue.recommendation.is_empty() {
                        output.push_str(&format!("      → {}\n", issue.recommendation));
                    }
                    if let (Some(current), Some(recommended)) = (&issue.current_value, &issue.recommended_value) {
                        output.push_str(&format!("      Current: {}, Recommended: {}\n", current, recommended));
                    }
                }
                output.push_str("\n");
            }
            
            // Medium Issues
            let medium_issues: Vec<_> = report.issues.iter()
                .filter(|i| i.severity == pg_query_advisor::config_tuner::models::ValidationSeverity::Medium)
                .collect();
            
            if !medium_issues.is_empty() {
                output.push_str(&format!("{} ({}):\n", "Medium".yellow(), medium_issues.len()));
                for issue in medium_issues {
                    output.push_str(&format!("   {} [{}] {}: {}\n",
                        "⚠️ ".yellow(),
                        issue.category,
                        issue.title,
                        issue.description
                    ));
                    if !issue.recommendation.is_empty() {
                        output.push_str(&format!("      → {}\n", issue.recommendation));
                    }
                    if let (Some(current), Some(recommended)) = (&issue.current_value, &issue.recommended_value) {
                        output.push_str(&format!("      Current: {}, Recommended: {}\n", current, recommended));
                    }
                }
                output.push_str("\n");
            }
            
            // Low Issues  
            let low_issues: Vec<_> = report.issues.iter()
                .filter(|i| i.severity == pg_query_advisor::config_tuner::models::ValidationSeverity::Low)
                .collect();
            
            if !low_issues.is_empty() {
                output.push_str(&format!("{} ({}):\n", "Low".bright_black(), low_issues.len()));
                for issue in low_issues {
                    output.push_str(&format!("   {} [{}] {}: {}\n",
                        "ℹ️ ".bright_black(),
                        issue.category,
                        issue.title,
                        issue.description
                    ));
                    if !issue.recommendation.is_empty() {
                        output.push_str(&format!("      → {}\n", issue.recommendation));
                    }
                }
                output.push_str("\n");
            }
            
            // Summary
            output.push_str(&format!("{}\n", "━".repeat(70).bright_blue()));
            output.push_str(&format!("Summary: {}\n", report.summary()));
            
            output
        }
        OutputFormat::Json => {
            serde_json::to_string_pretty(&report)?
        }
        OutputFormat::Markdown => {
            let mut output = String::new();
            output.push_str("# PostgreSQL Configuration Validation Report\n\n");
            
            if report.is_healthy() {
                output.push_str("✅ **Status:** Healthy\n\n");
            } else {
                output.push_str("⚠️ **Status:** Issues Detected\n\n");
            }
            
            // Group issues by severity
            for severity in [
                pg_query_advisor::config_tuner::models::ValidationSeverity::Critical,
                pg_query_advisor::config_tuner::models::ValidationSeverity::High,
                pg_query_advisor::config_tuner::models::ValidationSeverity::Medium,
                pg_query_advisor::config_tuner::models::ValidationSeverity::Low,
            ] {
                let issues: Vec<_> = report.issues.iter()
                    .filter(|i| i.severity == severity)
                    .collect();
                
                if !issues.is_empty() {
                    output.push_str(&format!("## {} Issues ({})\n\n", severity, issues.len()));
                    for issue in issues {
                        output.push_str(&format!("- **[{}] {}:** {}\n", 
                            issue.category, issue.title, issue.description));
                        if !issue.recommendation.is_empty() {
                            output.push_str(&format!("  - Recommendation: {}\n", issue.recommendation));
                        }
                        if let (Some(current), Some(recommended)) = (&issue.current_value, &issue.recommended_value) {
                            output.push_str(&format!("  - Current: `{}`\n", current));
                            output.push_str(&format!("  - Recommended: `{}`\n", recommended));
                        }
                    }
                    output.push_str("\n");
                }
            }
            
            output
        }
    };
    
    // Output
    if let Some(output_path) = opts.output {
        std::fs::write(&output_path, &output_text)?;
        println!("\n{} Report saved to: {}", 
            "✓".bright_green(), 
            output_path.display().to_string().bright_white());
    } else {
        print!("{}", output_text);
    }
    
    // Exit with error if not healthy
    if !report.is_healthy() {
        std::process::exit(1);
    }
    
    Ok(())
}
