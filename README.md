# Vaanyze - AI-Powered PostgreSQL Fleet Manager

A fast, intelligent platform that analyzes your PostgreSQL databases and provides actionable recommendations to improve query performance and reduce costs. Now with a **comprehensive web dashboard** and AI-powered features!

## 🌟 What's New

### ☸️ NEW: CloudNativePG Plugin (Beta v0.1.0)
- **Kubernetes-Native**: Deploy as a CloudNativePG (CNPG) plugin
- **CNPG-I gRPC**: Implements CNPG-I `Operator` service endpoints
- **Safe Tuning**: Returns JSONPatch to add tuned `.spec.postgresql.parameters` (opt-in, non-overwriting)
- **Health Checks**: `/health` and `/ready` endpoints for Kubernetes probes
- **Container-Ready**: Dedicated `Dockerfile.cnpg`
- **CI/CD**: CNPG-focused workflow (build/test + Docker + optional Kind readiness)

### 🎯 AI-Powered Web Dashboard (Phase 5)
- **Fleet Management**: Monitor all your databases in one beautiful interface
- **AI Chat Assistant**: Ask questions in plain English, get expert answers instantly
- **Query Optimizer UI**: Paste slow queries, get AI-optimized versions with explanations
- **Config Tuning Wizard**: Auto-generate optimal PostgreSQL configurations
- **Lock Analysis**: Real-time eBPF-powered lock contention tracking with charts
- **Cost Tracking**: See potential monthly savings across your entire fleet
- **Zero Build Step**: Pure HTML/CSS/JS - just open and use!

### Multi-Provider AI Integration 🧠
- **Multiple LLM Providers**: OpenAI (GPT-4), Anthropic (Claude), Ollama (local models)
- **Automatic AI Recommendations**: AI-powered analysis integrated into core workflow
- **Natural Language Ask**: Query your database health in plain English
- **Context-Aware Insights**: AI considers your schema, query patterns, and workload

### Phase 2 Complete: Advanced Index Intelligence ✨
- **5 Index Types**: BTREE, HASH, GIN, GIST, BRIN (not just BTREE!)
- **Smart Detection**: Automatically selects optimal index type based on data characteristics
- **97% Size Reduction**: BRIN indexes for sequential data (timestamps, auto-increment IDs)
- **Partial Indexes**: For skewed data distributions (e.g., 95% "completed" status)
- **Operator Classes**: jsonb_path_ops, gin_trgm_ops for specialized use cases

### Other Features
- 🤖 **Query Rewriting**: Automatic query optimization (flatten subqueries, add LIMIT, etc.)
- 🔄 **Auto-Apply**: Safely apply recommendations with automatic rollback on failure
- 📊 **Agent Mode**: Continuous monitoring with Slack/email alerts
- 💾 **Historical Tracking**: Track improvements over time

## Features

- 🚀 **Fast Analysis** - Analyzes databases in seconds
- 🧠 **Intelligent** - Uses PostgreSQL's own statistics (pg_stats) for data-driven decisions
- 🤖 **AI-Powered** - Optional LLM integration for deeper insights (OpenAI, Claude, Ollama)
- 💰 **Cost Estimation** - Shows potential monthly savings in dollars
- 🎯 **Actionable** - Generates ready-to-run SQL migration scripts
- 🔒 **Safe** - All index operations use `CONCURRENTLY` (no table locking)
- 🤖 **Automated** - Auto-apply mode with verification and rollback
- 📈 **Continuous** - Agent mode for 24/7 monitoring

## What It Detects

### Index Optimization (NEW: 5 Types!)
- **BTREE** - General purpose indexes (equality + range queries)
- **HASH** - High cardinality columns with equality-only queries (smaller than BTREE)
- **GIN** - JSONB columns, arrays, full-text search
- **GIST** - Geometric/spatial data (PostGIS)
- **BRIN** - Sequential data on large tables (97% smaller than BTREE!)
- **Partial Indexes** - For skewed distributions (WHERE clause to exclude common values)

### Query Issues
- **Slow queries** that need optimization
- **Missing indexes** on frequently queried columns
- **Unused indexes** wasting storage
- **High-volume queries** that could benefit from caching
- **Subqueries** that can be flattened to JOINs
- **SELECT *** that should use explicit columns

### Auto-Apply Features
- Safe execution with baseline capture
- Automatic verification of improvements
- Rollback on degradation
- Slack notifications for changes

## Quick Start

### Option 1: Web Dashboard (Recommended)

```bash
# 1. Build the project
cargo build --release

# 2. Set AI provider (choose one)
export OPENAI_API_KEY="sk-..."        # OpenAI (best quality)
# OR
export ANTHROPIC_API_KEY="sk-ant-..."  # Anthropic Claude
# OR
ollama pull llama3.2                   # Ollama (free, local)
export DBO_AI_PROVIDER="ollama"

export DBO_AI_ENABLED=true

# 3. Start HTTP server
./target/release/pg-query-advisor \
  --http-server \
  --http-port 8080 \
  --database-url "postgresql://user:pass@localhost/yourdb"

# 4. Open your browser
# http://localhost:8080/dashboard.html
```

**That's it!** You now have:
- ✅ AI-powered fleet dashboard
- ✅ Query optimizer with instant results
- ✅ Config tuning wizard
- ✅ Real-time lock analysis
- ✅ AI chat assistant

### Option 2: CLI Tool

```bash
# From source (requires Rust)
cargo build --release

# Binary will be at: target/release/pg-query-advisor
```

### Prerequisites

Your PostgreSQL database must have `pg_stat_statements` extension enabled:

```sql
-- Run as superuser
CREATE EXTENSION IF NOT EXISTS pg_stat_statements;

-- Optional: For advanced index analysis
ANALYZE;  -- Update pg_stats
```

Then restart your application to start collecting statistics.

### CLI Usage

```bash
# Analyze database
./target/release/pg-query-advisor --database-url postgresql://user:pass@localhost/mydb

# With advanced index intelligence
pg-query-advisor -d $DATABASE_URL --advanced-indexes

# Enable query rewriting suggestions
pg-query-advisor -d $DATABASE_URL --enable-rewriter

# Auto-apply safe recommendations
pg-query-advisor -d $DATABASE_URL --auto-apply --auto-approve low

# Agent mode (continuous monitoring)
pg-query-advisor agent \
  --database-url $DATABASE_URL \
  --check-interval 3600 \
  --slack-webhook $SLACK_WEBHOOK

# Export to JSON
pg-query-advisor -d $DATABASE_URL --format json --output report.json

# HTTP Server mode (for web dashboard)
pg-query-advisor --database-url $DATABASE_URL serve --port 8080

# With eBPF lock tracking (Linux only, requires CAP_BPF)
pg-query-advisor --database-url $DATABASE_URL serve --port 8080 --enable-ebpf
```

### eBPF Setup (Linux Only)

eBPF provides kernel-level lock contention tracking with sub-millisecond precision:

```bash
# 1. Requirements: Linux 5.8+ kernel with BTF support
uname -r  # Should be >= 5.8

# 2. Grant capabilities (instead of running as root)
sudo setcap cap_bpf,cap_perfmon=eip ./target/release/pg-query-advisor

# 3. Build the eBPF object files
make -C bpf
ls bpf/.output/lock_contention.bpf.o  # should exist

# 4. Start with eBPF enabled
./target/release/pg-query-advisor \
  --database-url $DATABASE_URL \
  serve --port 8080 --enable-ebpf

# The dashboard will show:
# - Real-time lock wait times (µs precision)
# - Futex contention events
# - Lock hotspots by table/query
```

**Without eBPF:** Lock analysis falls back to `pg_stat_activity` (still useful, but less precise).

### CloudNativePG Plugin (Kubernetes)

Deploy Vaanyze as a native CloudNativePG operator plugin for PostgreSQL configuration tuning:

```bash
# 1. Install CloudNativePG operator (if not already installed)
kubectl apply -f https://raw.githubusercontent.com/cloudnative-pg/cloudnative-pg/release-1.22/releases/cnpg-1.22.1.yaml

# 2. Deploy Vaanyze CNPG plugin
kubectl apply -f k8s/cnpg-plugin.yaml

# 3. Inspect plugin logs
kubectl logs -n cnpg-system -l app=vaanyze-cnpg-plugin --tail=50
```

**What it does:**
- ✅ Implements the CNPG-I gRPC plugin interface (Operator → Plugin calls)
- ✅ Adds tuned PostgreSQL parameters under `.spec.postgresql.parameters` via JSONPatch
- ✅ Safe-by-default: opt-in (`vaanyze.io/enable-tuning: "true"`) and **never overwrites** existing parameter keys
- ✅ Reports health via Kubernetes probes (`/health`, `/ready`)
- ✅ Designed to work alongside CNPG’s built-in monitoring (complementary)

**How it's different from CNPG monitoring:**
- **CNPG provides:** Metrics (WAL stats, replication lag, query counts) → "What's happening?"
- **Vaanyze provides:** Safe config tuning policy → "What should I set/change?"
- **Together:** CNPG alerts show symptoms → Vaanyze can propose safe parameter adjustments

See [CNPG_QUICK_START.md](CNPG_QUICK_START.md) for detailed setup instructions.

### Web Dashboard Usage

Once the HTTP server is running, access these features:

**Fleet Management:**
```
http://localhost:8080/dashboard.html
```

**API Endpoints:**
```bash
# Register a database to monitor
curl -X POST http://localhost:8080/api/v1/fleet \
  -H "Content-Type: application/json" \
  -d '{
    "name": "Production DB",
    "connection_string": "postgresql://...",
    "environment": "production"
  }'

# Ask AI a question
curl -X POST http://localhost:8080/api/v1/ai/ask \
  -H "Content-Type: application/json" \
  -d '{
    "question": "Why is my database slow?",
    "context": "5 databases, 3 critical issues"
  }'

# Optimize a query with AI
curl -X POST http://localhost:8080/api/v1/ai/optimize \
  -H "Content-Type: application/json" \
  -d '{"query": "SELECT * FROM users WHERE..."}'
```

### AI-Powered Analysis

```bash
# Analyze with AI recommendations (uses OpenAI by default)
pg-query-advisor -d $DATABASE_URL --ai --ai-api-key $OPENAI_API_KEY

# Use Anthropic Claude for AI analysis
pg-query-advisor -d $DATABASE_URL --ai --ai-provider anthropic --ai-api-key $ANTHROPIC_API_KEY

# Use local Ollama (no API key needed)
pg-query-advisor -d $DATABASE_URL --ai --ai-provider ollama --ollama-url http://localhost:11434

# Ask natural language questions about your database
pg-query-advisor ask "What are the slowest queries and how can I optimize them?" \
  --database-url $DATABASE_URL \
  --ai-provider openai \
  --ai-api-key $OPENAI_API_KEY

# Agent mode with AI recommendations
pg-query-advisor agent \
  --database-url $DATABASE_URL \
  --ai --ai-provider anthropic \
  --ai-api-key $ANTHROPIC_API_KEY
```

**Supported LLM Providers:**

| Provider | Models | API Key Env Var |
|----------|--------|-----------------|
| OpenAI | gpt-4-turbo-preview, gpt-4, gpt-3.5-turbo | `OPENAI_API_KEY` |
| Anthropic | claude-3-sonnet-20240229, claude-3-opus | `ANTHROPIC_API_KEY` |
| Ollama | llama2, mistral, codellama (local) | None (local) |

## Advanced Features

### 1. Advanced Index Intelligence (NEW!)

Automatically selects the optimal index type based on your data:

```bash
pg-query-advisor -d $DATABASE_URL --advanced-indexes
```

**Example Output:**

```sql
-- JSONB Column → GIN with operator class
CREATE INDEX CONCURRENTLY idx_orders_metadata_gin
  ON public.orders USING gin (metadata jsonb_path_ops);
-- Benefit: 85% faster JSONB containment queries

-- Sequential Timestamp → BRIN (97% smaller!)
CREATE INDEX CONCURRENTLY idx_events_created_at_brin
  ON public.events USING brin (created_at);
-- Size: 4.8MB vs 150MB BTREE index

-- Skewed Status Column → Partial Index
CREATE INDEX CONCURRENTLY idx_orders_status_btree
  ON public.orders USING btree (status)
  WHERE status != 'completed';
-- Size: 95% reduction by excluding dominant value

-- Text Search → GIN with trigrams
CREATE INDEX CONCURRENTLY idx_products_name_gin
  ON public.products USING gin (name gin_trgm_ops);
-- Benefit: Fast LIKE '%pattern%' queries
```

**Detection Logic:**
- Analyzes `pg_stats` for cardinality, correlation, null fraction
- Checks `pg_stat_statements` for query patterns
- Detects: JSON ops, array ops, text patterns, geometric ops
- Recommends optimal index type + operator class

### 2. Query Rewriting

Automatically optimizes query structure:

```bash
pg-query-advisor -d $DATABASE_URL --enable-rewriter
```

**Transformations:**
- `SELECT *` → Explicit column list
- Subqueries → JOINs (when beneficial)
- Predicate pushdown to reduce scans
- Add LIMIT to unbounded queries
- Remove redundant JOINs

**Example:**

```sql
-- Before
SELECT * FROM users WHERE id IN (SELECT user_id FROM orders WHERE total > 100)

-- After (82% faster)
SELECT users.id, users.email, users.name 
FROM users 
INNER JOIN orders ON users.id = orders.user_id 
WHERE orders.total > 100
```

### 3. Auto-Apply Mode

Safely apply recommendations automatically:

```bash
pg-query-advisor -d $DATABASE_URL \
  --auto-apply \
  --auto-approve low \  # Auto-approve low-risk changes
  --slack-webhook $WEBHOOK
```

**Safety Features:**
- Captures baseline metrics before changes
- Verifies improvement after applying
- Automatic rollback if performance degrades
- Risk assessment (low/medium/high)
- Slack notifications with details

### 4. Agent Mode (Continuous Monitoring)

Run as a background service:

```bash
pg-query-advisor agent \
  --database-url $DATABASE_URL \
  --check-interval 3600 \
  --slack-webhook $WEBHOOK \
  --state-file /var/lib/pg-advisor/state.json
```

**Features:**
- Periodic health checks
- Track applied recommendations
- Alert on new issues
- Historical trend analysis
- JSON state persistence

### 5. eBPF Kernel-Level Profiling (NEW!)

Get deep insights with kernel-level instrumentation:

```bash
# Enable eBPF mode (requires Linux with CAP_BPF)
pg-query-advisor -d $DATABASE_URL --ebpf

# With container-aware metrics (Docker/Kubernetes)
pg-query-advisor -d $DATABASE_URL --ebpf --container-aware

# Full correlation mode
pg-query-advisor -d $DATABASE_URL --ebpf --full-correlation
```

**eBPF Capabilities:**

| Feature | Description |
|---------|-------------|
| **I/O Latency Tracking** | Per-query syscall-level read/write latency (p50/p95/p99) |
| **WAL Correlation** | Correlate I/O spikes with WAL writes and checkpoints |
| **Lock Contention** | Track futex waits and lock holder/waiter chains |
| **Vacuum Detection** | Identify autovacuum processes and their I/O impact |
| **Container Metrics** | cgroup v1/v2 CPU/memory/IO attribution |
| **Session Tracking** | Query correlation across connection pools |
| **Graceful Degradation** | Falls back to pg_stat when eBPF unavailable |

**Example Output:**

```
Query: SELECT * FROM orders WHERE user_id = $1
  pg_stat: 1.2ms mean, 15,000 calls
  eBPF:    0.8ms CPU, 0.3ms I/O wait, 0.1ms lock wait
  Correlation: 95% confidence
  Root Cause: 25% time in sequential scan I/O
  Recommendation: CREATE INDEX on user_id (estimated 80% improvement)
```

**Requirements:**
- Linux kernel 5.8+ (for full features) or 4.15+ (basic)
- CAP_BPF and CAP_PERFMON capabilities, or privileged daemon mode
- BTF enabled kernel (for CO-RE portability)

## Command Line Options

### Main Command

```
Options:
  -d, --database-url <DATABASE_URL>
          PostgreSQL connection string [env: DATABASE_URL]
  
  -f, --format <FORMAT>
          Output format [default: terminal] [possible values: terminal, json, markdown]
  
  -o, --output <OUTPUT>
          Output file (defaults to stdout)
  
  -n, --query-limit <QUERY_LIMIT>
          Number of queries to analyze (top N by total time) [default: 100]
  
  -c, --cost-per-hour <COST_PER_HOUR>
          Database compute cost per hour (USD) for cost estimation [default: 0.07]
  
  -s, --min-severity <MIN_SEVERITY>
          Minimum severity to include in report [default: low]
  
  --advanced-indexes
          Enable advanced index type selection (BRIN, GIN, GIST, HASH)
  
  --enable-rewriter
          Enable query rewriting suggestions
  
  --auto-apply
          Automatically apply safe recommendations
  
  --auto-approve <RISK_LEVEL>
          Auto-approve recommendations up to risk level [possible values: low, medium, high]
  
  --slack-webhook <URL>
          Slack webhook for notifications
  
  -v, --verbose
          Enable verbose logging
  
  -h, --help
          Print help
```

### Agent Command

```bash
pg-query-advisor agent [OPTIONS]

Options:
  --check-interval <SECONDS>     Time between checks [default: 3600]
  --state-file <PATH>            Path to state file [default: ~/.pg-advisor/state.json]
  --slack-webhook <URL>          Slack webhook URL
  --email-to <EMAIL>             Email address for alerts
  --email-from <EMAIL>           From email address
  --smtp-host <HOST>             SMTP server host
  --smtp-port <PORT>             SMTP server port [default: 587]
```

## Example Output

```
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
PostgreSQL Query Health Advisor v0.3.0
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
Database: production_db
Analyzed: 2024-11-02 15:00:00 UTC
Mode: Advanced Index Intelligence + Query Rewriting

SUMMARY
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
  18 Issues Found
  ● 3 Critical  ● 7 High Priority  ● 5 Medium  ● 3 Low

  6 Slow Queries
  8 Advanced Index Recommendations
  2 Query Rewrite Suggestions
  2 Unused Indexes Found

  💰 Potential Savings: $1,240/month

TOP RECOMMENDATIONS
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

1. [CRITICAL] Create BRIN index on events.created_at (NEW!)
   Column shows strong sequential pattern (correlation: 0.96) on large table (10M rows).
   BRIN provides 100x smaller index size with good performance for range queries.
   
   💰 Est. savings: $420/month
   📊 Index type: BRIN (4.8MB vs 150MB BTREE)
   🎯 Confidence: 90%
   
   Migration SQL:
   CREATE INDEX CONCURRENTLY idx_events_created_at_brin
     ON public.events USING brin (created_at);
   
   ℹ️  Index type: BRIN. Estimated size: 4.8MB. Confidence: 90%

2. [HIGH] Create GIN index on orders.metadata (NEW!)
   GIN index recommended for JSONB column 'metadata' with JSON operations.
   Using jsonb_path_ops for smaller index size and faster containment queries.
   
   💰 Est. savings: $280/month
   📊 Affects 15,000 queries
   🎯 Confidence: 95%
   
   Migration SQL:
   CREATE INDEX CONCURRENTLY idx_orders_metadata_gin
     ON public.orders USING gin (metadata jsonb_path_ops);

3. [HIGH] Flatten subquery to JOIN (NEW!)
   Query can be optimized by converting IN subquery to INNER JOIN.
   
   💰 Est. savings: $180/month
   📊 Current: 450ms → Optimized: 80ms (82% faster)
   
   Original Query:
   SELECT * FROM users WHERE id IN (SELECT user_id FROM orders WHERE total > 100)
   
   Optimized Query:
   SELECT users.id, users.email, users.name 
   FROM users 
   INNER JOIN orders ON users.id = orders.user_id 
   WHERE orders.total > 100
   
   Applied rules:
     ✓ Remove SELECT *
     ✓ Flatten subquery to JOIN

4. [MEDIUM] Create partial BTREE index on orders.status (NEW!)
   Partial BTREE index recommended for low cardinality column 'status'.
   Dominant value appears in 95.0% of rows.
   WHERE clause 'status != 'completed'' reduces index size by ~95%.
   
   💰 Est. savings: $120/month
   📊 Index size: 5MB (vs 100MB full index)
   
   Migration SQL:
   CREATE INDEX CONCURRENTLY idx_orders_status_btree
     ON public.orders USING btree (status)
     WHERE status != 'completed';

...
```

## How It Works

### Analysis Pipeline

1. **Collection** - Gathers data from PostgreSQL system views:
   - `pg_stat_statements` - Query statistics
   - `pg_stats` - Column distribution data
   - `pg_stat_user_indexes` - Index usage
   - `pg_class` - Table sizes

2. **Advanced Analysis** (NEW):
   - **Column Statistics**: Cardinality, correlation, null fraction
   - **Query Pattern Detection**: JSON ops, array ops, text patterns
   - **Index Type Selection**: Decision tree based on data characteristics
   - **Query Structure Analysis**: Subquery detection, predicate analysis

3. **Recommendation Generation**:
   - Risk assessment (low/medium/high)
   - Cost-benefit analysis
   - DDL generation with optimal parameters
   - Rollback SQL included

4. **Output**:
   - Terminal (colored, formatted)
   - JSON (machine-readable)
   - Markdown (documentation)

## Safety

- **Read-only by default** - Never modifies your database unless `--auto-apply` is used
- **Safe migrations** - All index operations use `CONCURRENTLY`
- **Includes rollback** - Every recommendation includes rollback SQL
- **Verification** - Auto-apply mode verifies improvements before committing
- **Automatic rollback** - Reverts changes if performance degrades
- **No sensitive data** - Only collects query patterns and statistics

## Performance

- Analyze 100 queries: **< 5 seconds**
- Analyze 1000 queries: **< 15 seconds**
- Advanced index analysis: **+2-3 seconds per table**
- Memory usage: **< 50MB**
- Zero performance impact on production database

## Architecture

The tool is organized into several modules:

### Core Modules
- **models.rs** - Core data structures
- **collector.rs** - Collects statistics from PostgreSQL
- **analyzer.rs** - Basic heuristic analysis
  - **analyzer/advanced/** - Advanced index intelligence (NEW!)
- **cost_estimator.rs** - Estimates cost savings
- **recommendations.rs** - Orchestrates analysis
- **output.rs** - Formats output

### Advanced Features
- **query_rewriter/** - Query optimization engine
- **auto_apply/** - Safe automatic application
- **agent.rs** - Continuous monitoring
- **config.rs** - Configuration management

## Limitations

- Requires `pg_stat_statements` extension
- Works best with PostgreSQL 12+ (11 supported with limited features)
- BRIN indexes require sequential insert patterns
- Cost estimates are approximations based on compute time
- Advanced index intelligence requires `ANALYZE` to be run regularly

## Development

```bash
# Build
cargo build --release

# Run tests (unit tests)
cargo test

# Run integration tests (requires PostgreSQL)
TEST_DATABASE_URL=postgresql://user:pass@localhost/test cargo test -- --ignored

# Run advanced index tests
cargo test --lib index_type_selector

# Format code
cargo fmt

# Lint
cargo clippy
```

## Benchmarks

Performance improvements from using advanced index intelligence:

| Scenario | Before | After | Improvement |
|----------|--------|-------|-------------|
| JSONB containment queries | 450ms | 65ms | 85% faster |
| Sequential range scan (10M rows) | 2.3s | 180ms | 92% faster |
| Text pattern matching (LIKE) | 890ms | 95ms | 89% faster |
| Index storage (timestamps) | 150MB | 4.8MB | 97% smaller |
| Partial index (skewed data) | 100MB | 5MB | 95% smaller |

## Comparison to Alternatives

**Vaanyze vs Observability Tools:** Vaanyze is an **AI-powered optimization platform**, not just an observability tool. It's complementary to monitoring solutions like CloudNativePG, pgNow, pganalyze, and Grafana.

### How Vaanyze is Different

**Observability Tools** (CNPG, pgNow, pganalyze, Grafana):
- **What they do:** Show you metrics, dashboards, alerts
- **Focus:** "What's happening?" - Current state visibility
- **Action required:** Manual interpretation and fixes
- **Examples:** Query execution times, replication lag, connection counts

**Vaanyze - Autonomous Optimization**:
- **What it does:** Analyzes patterns and **automatically fixes** problems
- **Focus:** "How to optimize?" - AI-powered recommendations + auto-apply
- **Action:** Autonomous with safety guarantees (auto-rollback)
- **Examples:** "Add index X → saves $120/month", then applies it safely

**They work together:** 
- CNPG/pgNow/Grafana **detect** the problem → Vaanyze **fixes** it automatically
- Monitoring shows "query slow" → Vaanyze creates index and validates improvement

### Feature Comparison

| Feature | Vaanyze | pgNow | CloudNativePG | pganalyze |
|---------|---------|-------|---------------|-----------|
| **Purpose** | AI Optimization | Diagnostics | Monitoring | Analytics |
| **Index Types** | 5 (BTREE, HASH, GIN, GIST, BRIN) | Basic stats | Metrics only | 1 (BTREE) |
| **Missing Index Detection** | ✅ Yes + Auto-create | ❌ No | ❌ No | ✅ Yes |
| **Unused Index Detection** | ✅ Yes + Auto-drop | Limited | ❌ No | ✅ Yes |
| **Query Rewriting** | ✅ AI-powered | ❌ No | ❌ No | ❌ No |
| **Auto-Apply** | ✅ With rollback | ❌ No | ❌ No | ❌ Manual |
| **AI/LLM Integration** | ✅ Multi-provider | ❌ No | ❌ No | ❌ No |
| **eBPF Profiling** | ✅ Kernel-level | ❌ No | ❌ No | ❌ No |
| **Vacuum Analysis** | ✅ Real-time eBPF | ✅ Reporting | ✅ Metrics | ✅ Yes |
| **Live Session Monitor** | ✅ eBPF + API | ✅ Desktop UI | ✅ Metrics | ✅ Yes |
| **Cost Estimation** | ✅ Per-recommendation | ❌ No | ❌ No | ❌ No |
| **CloudNativePG Plugin** | ✅ Native gRPC | ❌ No | ✅ Built-in | ❌ No |
| **Kubernetes Native** | ✅ CNPG plugin | ❌ No | ✅ Operator | ✅ Helm |
| **Architecture** | Server-side daemon | Desktop app | Operator | SaaS |
| **Deployment** | K8s/Docker/Binary | Desktop install | K8s only | Cloud |
| **Price** | Free (MIT) | Free | Free (Apache) | $150+/mo |

**Use Vaanyze when:**
- You want **autonomous optimization** not just monitoring
- You need **AI-powered recommendations** with automatic fixes
- You want to **reduce cloud costs** with proven ROI tracking
- You manage **multiple databases** (fleet management)
- You need **Kubernetes-native** optimization (CNPG plugin)

**Use pgNow/CNPG/Grafana when:**
- You need **dashboards** and visualizations
- You want **ad-hoc debugging** with desktop UI
- You prefer **manual control** over all changes
- You need **historical metrics** for compliance

**Best approach:** Use **both together**
- CloudNativePG/Grafana: Monitoring + alerting
- Vaanyze: Automated optimization + cost reduction
- pgNow: Ad-hoc debugging when needed

## Roadmap

### ✅ Phase 1 Complete - MVP
- Basic analysis engine
- Index recommendations (BTREE only)
- Cost estimation
- JSON/Markdown export

### ✅ Phase 2 Complete - Advanced Features
- Advanced Index Intelligence (5 types!)
- Query rewriting
- Auto-apply with rollback
- Agent mode
- Slack/email notifications

### ✅ Phase 3 Complete - eBPF Integration
- **Kernel-level profiling** with syscall-level I/O latency tracking
- **Root cause analysis** - I/O vs CPU vs lock contention breakdown
- **WAL correlation** - Correlate I/O with WAL writes and checkpoints
- **Vacuum detection** - Identify and track autovacuum impact
- **Container-aware metrics** - cgroup v1/v2 support for Docker/Kubernetes
- **Graceful degradation** - Automatic fallback when eBPF unavailable
- **Live map updates** - Hot-reload sampling rates without restart
- **Full pg_stat correlation** - Combine eBPF metrics with pg_stat_statements
- **Session tracking** - Query correlation across connection pools
- **Query normalization** - Consistent hashing for parameterized queries

### ✅ Phase 4 Complete - AI Integration
- **Multi-provider LLM support** - OpenAI, Anthropic Claude, Ollama
- **Automatic AI recommendations** - Integrated into analyze and agent modes
- **Natural language queries** - Ask questions in plain English via `ask` command
- **Context-aware analysis** - AI considers schema, query patterns, and workload
- **Local model support** - Run with Ollama for privacy-sensitive environments

### ✅ Phase 5 Complete - Web Dashboard & Fleet Management
- **AI-powered web dashboard** - Modern, responsive UI with real-time updates
- **Fleet management** - Monitor multiple databases from one interface
- **AI chat assistant** - Natural language Q&A about your databases
- **Query optimizer UI** - Visual query optimization with AI explanations
- **Config tuning wizard** - Auto-generate optimal PostgreSQL configs
- **Lock analysis visualization** - Real-time charts and blocking query detection
- **Cost tracking** - Aggregate potential savings across fleet
- **HTTP API** - RESTful API for programmatic access
- **Chart.js integration** - Interactive time-series visualizations

### ✅ Phase 5.5 Complete - CloudNativePG Integration
- **CNPG Plugin** - Native CloudNativePG operator plugin (v0.1.0-beta)
- **gRPC Protocol** - CNPG-I protocol implementation
- **Kubernetes Deployment** - Production-ready manifests with health probes
- **Docker Images** - Multi-stage builds with eBPF support
- **CI/CD Pipeline** - Automated testing and container builds
- **Documentation** - Complete quick-start guide and integration docs

### 📅 Phase 6 Planned - Enterprise Features
- Team collaboration and RBAC
- Historical trends and analytics (7/30/90 days)
- Grafana/Prometheus dashboards
- Multi-tenant support
- Cloud marketplace listings (AWS, Azure, GCP)
- Custom AI training on your query patterns

## Contributing

Contributions welcome! Areas where help is needed:

- Additional index type heuristics
- Query rewriting rules
- Performance benchmarks
- Documentation improvements
- Integration tests

Please open an issue first to discuss changes.

## License

MIT

## Support

- **GitHub Issues**: Bug reports and feature requests
- **Documentation**: See `docs/` directory
- **Examples**: Check `docs/ADVANCED_INDEX_IMPLEMENTATION_COMPLETE.md`

## Acknowledgments

Built with:
- Rust + Tokio (async runtime)
- sqlx (PostgreSQL driver)
- sqlparser-rs (SQL parsing)
- serde (serialization)

Inspired by:
- pganalyze
- pg_stat_statements
- PostgreSQL documentation
