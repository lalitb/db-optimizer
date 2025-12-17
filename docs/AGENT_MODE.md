# Agent Mode - Continuous Monitoring

## Overview
Agent mode runs pg-query-advisor as a background daemon that continuously monitors your database and automatically applies optimizations.

## How It Works

```
┌─────────────────────────────────────────────────────────┐
│                    Agent Mode                            │
├─────────────────────────────────────────────────────────┤
│                                                          │
│  1. Start Agent                                         │
│     └─> Runs as background process                      │
│                                                          │
│  2. Continuous Monitoring Loop                          │
│     ├─> Every N minutes (configurable)                  │
│     ├─> Query pg_stat_statements                        │
│     ├─> Analyze new/changed queries                     │
│     └─> Compare with previous run                       │
│                                                          │
│  3. Auto-Optimization                                   │
│     ├─> Detect new issues                               │
│     ├─> Apply safe fixes automatically                  │
│     ├─> Verify improvements                             │
│     └─> Rollback if needed                              │
│                                                          │
│  4. Alerting                                            │
│     ├─> Slack notifications                             │
│     ├─> Email alerts                                    │
│     └─> Metrics to Prometheus                           │
│                                                          │
└─────────────────────────────────────────────────────────┘
```

## Current Status

### ✅ Already Implemented
- Query collection from `pg_stat_statements`
- Performance analysis
- Auto-apply with verification
- Slack notifications
- Rollback on failure

### ⚠️ Needs Implementation
- Daemon/background mode
- Continuous monitoring loop
- State persistence (track what's been fixed)
- Scheduling (every N minutes)
- Graceful shutdown
- Health checks

## Quick Implementation

Here's what needs to be added to `src/main.rs`:

```rust
#[derive(Parser)]
struct Cli {
    // ... existing fields ...
    
    /// Run as background agent (continuous monitoring)
    #[arg(long)]
    agent: bool,
    
    /// Monitoring interval in minutes
    #[arg(long, default_value = "15")]
    interval_minutes: u64,
    
    /// State file to track applied optimizations
    #[arg(long, default_value = "~/.pg-advisor-state.json")]
    state_file: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    
    if cli.agent {
        // Run as agent
        run_agent_mode(cli).await?;
    } else {
        // Run once (existing behavior)
        run_once(cli).await?;
    }
    
    Ok(())
}

async fn run_agent_mode(cli: Cli) -> Result<()> {
    println!("🤖 Starting pg-query-advisor in agent mode...");
    println!("   Monitoring interval: {} minutes", cli.interval_minutes);
    println!("   Database: {}", cli.database_url);
    
    let mut interval = tokio::time::interval(
        tokio::time::Duration::from_secs(cli.interval_minutes * 60)
    );
    
    let state = AgentState::load(&cli.state_file)?;
    
    loop {
        interval.tick().await;
        
        println!("🔍 Running analysis cycle...");
        
        match run_analysis_cycle(&cli, &state).await {
            Ok(results) => {
                println!("✅ Analysis complete: {} optimizations applied", results.applied_count);
                state.save(&cli.state_file)?;
            }
            Err(e) => {
                eprintln!("❌ Analysis failed: {}", e);
                // Continue monitoring even if one cycle fails
            }
        }
    }
}

async fn run_analysis_cycle(cli: &Cli, state: &AgentState) -> Result<AnalysisResults> {
    // Connect to database
    let pool = PgPool::connect(&cli.database_url).await?;
    
    // Collect stats (existing code)
    let collector = StatsCollector::new(pool.clone());
    let queries = collector.collect_slow_queries().await?;
    let tables = collector.collect_table_stats().await?;
    let indexes = collector.collect_index_stats().await?;
    
    // Generate recommendations (existing code)
    let engine = RecommendationEngine::new(cli.cost_per_hour);
    let mut recommendations = engine.generate_recommendations(&queries, &tables, &indexes);
    
    // Filter out already-applied recommendations
    recommendations.retain(|rec| !state.is_applied(&rec.id));
    
    if recommendations.is_empty() {
        println!("   No new optimizations found");
        return Ok(AnalysisResults::default());
    }
    
    println!("   Found {} new optimization opportunities", recommendations.len());
    
    // Auto-apply if enabled
    if cli.auto_apply {
        let config = AutoApplyConfig {
            enabled: true,
            dry_run: cli.dry_run,
            // ... other config
        };
        
        let orchestrator = AutoApplyOrchestrator::new(pool, config);
        let results = orchestrator.process_all(&recommendations).await;
        
        // Track successful applications
        for result in &results {
            if matches!(result.status, ApplyStatus::Success) {
                state.mark_applied(&result.recommendation_id);
            }
        }
        
        Ok(AnalysisResults {
            applied_count: results.iter().filter(|r| matches!(r.status, ApplyStatus::Success)).count(),
            results,
        })
    } else {
        // Just report
        println!("   Auto-apply disabled. Run with --auto-apply to enable.");
        Ok(AnalysisResults::default())
    }
}

struct AgentState {
    applied_recommendations: HashSet<String>,
    last_run: DateTime<Utc>,
}

impl AgentState {
    fn load(path: &str) -> Result<Self> {
        // Load from file or create new
        if let Ok(content) = std::fs::read_to_string(path) {
            Ok(serde_json::from_str(&content)?)
        } else {
            Ok(Self {
                applied_recommendations: HashSet::new(),
                last_run: Utc::now(),
            })
        }
    }
    
    fn save(&self, path: &str) -> Result<()> {
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)?;
        Ok(())
    }
    
    fn is_applied(&self, id: &str) -> bool {
        self.applied_recommendations.contains(id)
    }
    
    fn mark_applied(&mut self, id: &str) {
        self.applied_recommendations.insert(id.to_string());
        self.last_run = Utc::now();
    }
}
```

## Usage Examples

### Start Agent
```bash
# Start monitoring every 15 minutes
pg-query-advisor \
  --database-url $DB_URL \
  --agent \
  --auto-apply \
  --interval-minutes 15 \
  --slack-webhook $SLACK_WEBHOOK

# Output:
🤖 Starting pg-query-advisor in agent mode...
   Monitoring interval: 15 minutes
   Database: postgresql://...
   
[2024-11-02 09:00:00] 🔍 Running analysis cycle...
[2024-11-02 09:00:05] ✅ Analysis complete: 3 optimizations applied
   - Added index idx_users_email (90% improvement)
   - Removed unused index idx_old (saved 45MB)
   - Flattened subquery in /api/dashboard (40% improvement)

[2024-11-02 09:15:00] 🔍 Running analysis cycle...
[2024-11-02 09:15:03] ✅ Analysis complete: 0 optimizations applied
   - No new issues found

[2024-11-02 09:30:00] 🔍 Running analysis cycle...
...
```

### Run as Systemd Service
```ini
# /etc/systemd/system/pg-advisor-agent.service
[Unit]
Description=PostgreSQL Query Advisor Agent
After=network.target postgresql.service

[Service]
Type=simple
User=postgres
Environment="DATABASE_URL=postgresql://..."
Environment="SLACK_WEBHOOK=https://hooks.slack.com/..."
ExecStart=/usr/local/bin/pg-query-advisor \
  --agent \
  --auto-apply \
  --interval-minutes 15 \
  --max-risk low \
  --min-savings 50
Restart=always
RestartSec=60

[Install]
WantedBy=multi-user.target
```

```bash
# Start service
sudo systemctl start pg-advisor-agent
sudo systemctl enable pg-advisor-agent

# Check status
sudo systemctl status pg-advisor-agent

# View logs
sudo journalctl -u pg-advisor-agent -f
```

### Run in Docker
```dockerfile
FROM rust:latest

WORKDIR /app
COPY . .
RUN cargo build --release

CMD ["./target/release/pg-query-advisor", \
     "--agent", \
     "--auto-apply", \
     "--interval-minutes", "15"]
```

```bash
docker run -d \
  --name pg-advisor-agent \
  -e DATABASE_URL=$DB_URL \
  -e SLACK_WEBHOOK=$SLACK_WEBHOOK \
  pg-query-advisor:latest
```

### Run in Kubernetes
```yaml
apiVersion: apps/v1
kind: Deployment
metadata:
  name: pg-advisor-agent
spec:
  replicas: 1
  template:
    spec:
      containers:
      - name: agent
        image: pg-query-advisor:latest
        args:
          - "--agent"
          - "--auto-apply"
          - "--interval-minutes=15"
        env:
          - name: DATABASE_URL
            valueFrom:
              secretKeyRef:
                name: db-creds
                key: connection-string
          - name: SLACK_WEBHOOK
            valueFrom:
              secretKeyRef:
                name: slack-creds
                key: webhook-url
```

## Benefits of Agent Mode

### Continuous Optimization
- Monitors 24/7
- Catches new issues as they arise
- No manual intervention needed

### Automatic Adaptation
- Learns from query patterns
- Adapts to workload changes
- Optimizes as app evolves

### Proactive Alerts
- Notified immediately of issues
- Slack/email integration
- Metrics for dashboards

### State Management
- Tracks what's been fixed
- Avoids re-applying same fixes
- Persistent across restarts

## Configuration File

```yaml
# ~/.pg-advisor-agent.yaml
agent:
  enabled: true
  interval_minutes: 15
  state_file: /var/lib/pg-advisor/state.json
  
database:
  url: postgresql://user:pass@localhost/db
  cost_per_hour: 0.15
  
auto_apply:
  enabled: true
  dry_run: false
  min_savings_threshold: 50.0
  max_risk_level: low
  verification_duration_secs: 300
  auto_rollback_on_failure: true
  
notifications:
  slack_webhook: https://hooks.slack.com/...
  email: ops@company.com
  
monitoring:
  prometheus_port: 9090
  health_check_port: 8080
```

## Monitoring & Observability

### Health Check Endpoint
```rust
// Add HTTP server for health checks
#[get("/health")]
async fn health() -> impl Responder {
    HttpResponse::Ok().json(json!({
        "status": "healthy",
        "last_run": state.last_run,
        "applied_count": state.applied_recommendations.len()
    }))
}
```

### Prometheus Metrics
```rust
// Expose metrics
lazy_static! {
    static ref OPTIMIZATIONS_APPLIED: Counter = Counter::new(
        "pg_advisor_optimizations_applied_total",
        "Total optimizations applied"
    ).unwrap();
    
    static ref ANALYSIS_DURATION: Histogram = Histogram::new(
        "pg_advisor_analysis_duration_seconds",
        "Analysis cycle duration"
    ).unwrap();
}
```

## Implementation Effort

**Estimated Time:** 4-6 hours

**Required Changes:**
1. Add `--agent` flag to CLI (30 min)
2. Implement agent loop (1 hour)
3. Add state persistence (1 hour)
4. Add health checks (30 min)
5. Add systemd/docker configs (1 hour)
6. Testing (1-2 hours)

**Dependencies:**
- `tokio` (already have)
- `serde_json` (already have)
- `signal-hook` (for graceful shutdown)

## Summary

**Current:** On-demand analysis tool  
**Future:** Continuous monitoring agent

**Key Point:** PostgreSQL's `pg_stat_statements` ALREADY tracks all queries from your application automatically. We just need to:
1. Run in a loop instead of once
2. Track what we've already fixed
3. Run as a daemon/service

The foundation is 100% there - just needs the agent wrapper! 🚀

Would you like me to implement this agent mode now?
