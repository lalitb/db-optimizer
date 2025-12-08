# Enhanced AI-Powered Fleet Dashboard - Setup & Usage Guide

## 🎯 Overview

The enhanced dashboard (`dashboard-enhanced.html`) is a **complete AI-powered interface** that integrates **all backend capabilities** of Vaanyze, including:

- 🤖 **AI Chat Assistant** - Natural language database Q&A
- 🚀 **AI Query Optimizer** - Instant query optimization with explanations
- ⚙️ **Config Tuning Wizard** - Auto-generate optimal PostgreSQL configs
- 🔒 **Lock Contention Analysis** - Real-time eBPF-powered lock tracking
- 🏢 **Fleet Management** - Multi-database monitoring with AI recommendations
- 💰 **Cost Savings Dashboard** - Track potential monthly savings

---

## 📋 Prerequisites

### 1. Backend Requirements

**Rust & Dependencies:**
```bash
# Ensure you have Rust 1.70+
rustc --version

# All dependencies are already in Cargo.toml
# Just build the project
cargo build --release
```

**PostgreSQL Extension:**
```sql
-- Connect to each database you want to monitor
CREATE EXTENSION IF NOT EXISTS pg_stat_statements;

-- Optional: For advanced analytics
ANALYZE;
```

### 2. AI Provider Setup

Choose ONE of the following AI providers:

#### Option A: OpenAI (Recommended - Best Quality)
```bash
export OPENAI_API_KEY="sk-..."
export DBO_AI_MODEL="gpt-4o-mini"  # or gpt-4o for better quality
export DBO_AI_ENABLED=true
```

**Cost:** ~$0.01 per query optimization, ~$0.005 per chat message

#### Option B: Anthropic Claude
```bash
export ANTHROPIC_API_KEY="sk-ant-..."
export DBO_AI_PROVIDER="anthropic"
export DBO_AI_MODEL="claude-3-haiku-20240307"
export DBO_AI_ENABLED=true
```

**Cost:** ~$0.008 per query, similar pricing to OpenAI

#### Option C: Ollama (Free - Local)
```bash
# Install Ollama first: https://ollama.ai
ollama pull llama3.2

export DBO_AI_PROVIDER="ollama"
export OLLAMA_HOST="http://localhost:11434"
export DBO_AI_MODEL="llama3.2"
export DBO_AI_ENABLED=true
```

**Cost:** Free! Runs locally, but slower and less accurate

---

## 🚀 Quick Start

### Step 1: Start the HTTP Server

```bash
# Set environment variables (example with OpenAI)
export OPENAI_API_KEY="sk-..."
export DATABASE_URL="postgresql://user:pass@localhost/yourdb"

# Build and run
cargo build --release

# Start HTTP server
./target/release/pg-query-advisor \
  --http-server \
  --http-port 8080 \
  --database-url $DATABASE_URL
```

### Step 2: Access the Dashboard

Open your browser to:
```
http://localhost:8080/dashboard-enhanced.html
```

### Step 3: Register Database Instances

**Via UI:**
1. Navigate to Fleet tab
2. Click "Add Instance" (coming soon in UI)

**Via API:**
```bash
curl -X POST http://localhost:8080/api/v1/fleet \
  -H "Content-Type: application/json" \
  -d '{
    "name": "Production DB",
    "connection_string": "postgresql://user:pass@prod-db:5432/myapp",
    "environment": "production",
    "tags": ["critical", "payments"]
  }'
```

**Via CLI:**
```bash
# Register multiple databases
./target/release/pg-query-advisor register-fleet \
  --name "Production DB" \
  --connection-string "postgresql://..." \
  --environment production

./target/release/pg-query-advisor register-fleet \
  --name "Staging DB" \
  --connection-string "postgresql://..." \
  --environment staging
```

---

## 🎨 Dashboard Features Walkthrough

### 1. Fleet Overview Tab 🏢

**What You See:**
- **Cost Savings Banner**: Total potential monthly savings across all databases
- **Fleet Summary Cards**: Total instances, healthy count, critical issues, avg health score
- **Instance Cards**: Click any database to see detailed AI recommendations

**Key Metrics Per Instance:**
- Health Score (0-100)
- Active Connections
- Lock Contention count
- Idle Transactions count
- eBPF Lock Wait time (if available)
- AI Recommendations count
- Estimated cost savings

**Actions:**
- **Click any instance card** → Opens detailed modal with:
  - Database info (version, size, table/index count)
  - AI-generated recommendations
  - One-click "Apply" or "Explain" buttons

**Example Use Case:**
```
User: Sees "Production DB" has health score of 62 (red)
Action: Clicks card
Result: Modal shows 3 AI recommendations:
  1. Create BRIN index on events.created_at → Save $420/mo
  2. Add index on users(email) → Save $180/mo
  3. Rewrite slow query → Save $95/mo
User: Clicks "Apply" on first recommendation
Result: Index is queued for safe application
```

---

### 2. Query Optimizer Tab 🚀

**What You See:**
- Split-pane interface: Original query (left) → Optimized query (right)
- Results panel showing:
  - Rules applied count
  - Estimated improvement %
  - Confidence score
  - Applied transformations list
  - AI explanation

**How to Use:**
1. Paste your slow SQL query in left pane
2. Click **"Optimize with AI"**
3. Wait 2-5 seconds
4. See optimized query in right pane
5. Click **"Explain Changes"** to understand what changed
6. Click **"Copy SQL"** to use in your app

**Example:**

**Input:**
```sql
SELECT * FROM orders
WHERE user_id IN (SELECT id FROM users WHERE status = 'active')
ORDER BY created_at DESC
```

**Output (after optimization):**
```sql
SELECT orders.id, orders.user_id, orders.total, orders.created_at
FROM orders
INNER JOIN users ON orders.user_id = users.id
WHERE users.status = 'active'
ORDER BY orders.created_at DESC
LIMIT 1000
```

**AI Explanation:**
- ✓ Removed `SELECT *` (saves bandwidth)
- ✓ Converted IN subquery to JOIN (87% faster)
- ✓ Added LIMIT to prevent unbounded results
- **Estimated improvement: 87%**

---

### 3. Config Tuning Tab ⚙️

**What You See:**
- Form to select database instance and override system specs
- Auto-detection of:
  - Workload type (OLTP, Data Warehouse, Web, Mixed)
  - System RAM
  - CPU cores
  - Disk type

**How to Use:**
1. Select database instance (or use auto-detect)
2. Optionally override RAM/CPU/Disk specs
3. Click **"Generate Optimal Configuration"**
4. Review recommended `postgresql.conf` changes
5. Click **"Download postgresql.conf"** or **"Copy to Clipboard"**
6. Apply to your database

**Example Output:**
```ini
# Optimized for: OLTP workload, 16GB RAM, 8 CPUs, SSD

# Connection Settings
max_connections = 200
shared_buffers = 4GB           # 25% of RAM
effective_cache_size = 12GB    # 75% of RAM

# Memory Settings
work_mem = 10MB                # RAM / max_connections / 2
maintenance_work_mem = 512MB

# WAL Settings
wal_buffers = 16MB
checkpoint_completion_target = 0.9

# Query Planner
random_page_cost = 1.1         # SSD optimized
effective_io_concurrency = 200

# Logging
log_min_duration_statement = 1000
log_line_prefix = '%t [%p]: '
```

**Workload Detection:**
- **OLTP**: High connection count, short queries, frequent writes
- **Data Warehouse**: Low connections, long queries, mostly reads
- **Web**: Moderate connections, mixed read/write
- **Mixed**: Doesn't fit clear pattern

---

### 4. Lock Analysis Tab 🔒

**What You See:**
- **Lock Stats Cards**:
  - Blocked Queries count
  - Total Wait Time
  - Lock Events (last 5 minutes)
- **Lock Wait Time Trend Chart**: Real-time graph using Chart.js
- **Top Blocking Queries Table**:
  - Query text
  - Number of queries it's blocking
  - Total wait time
  - Lock type
  - **Terminate button** (use carefully!)

**How to Use:**
1. Switch to Lock Analysis tab
2. Auto-refreshes every 10 seconds
3. Identify queries causing contention
4. Click **"Terminate"** to kill blocking query (confirms first)
5. Watch chart to see if contention drops

**eBPF vs Non-eBPF:**
- **With eBPF**: Kernel-level precision, futex tracking, sub-millisecond accuracy
- **Without eBPF**: pg_stat_activity only, less precise but still useful

**Example Alert:**
```
⚠️ High Lock Contention Detected

Blocking Query:
  UPDATE orders SET status = 'shipped' WHERE customer_id = 12345

Blocking: 47 queries
Wait Time: 2,347ms
Lock Type: RowExclusiveLock

Recommendation: Add index on orders(customer_id) to speed up UPDATE
```

---

### 5. AI Chat Assistant 🤖

**What You See:**
- Floating chat panel (bottom-right)
- Quick prompt buttons for common questions
- Chat history with AI responses

**How to Use:**
1. Click **"AI Assistant"** button in navbar
2. Type your question or click a quick prompt
3. AI analyzes your fleet and responds in ~2-5 seconds

**Example Conversations:**

**Q:** "Why is my database slow?"

**AI:** "Based on your fleet analysis, I found 3 main issues:
1. Production DB has 47 locked queries waiting on a single UPDATE statement
2. Staging DB is missing indexes on users(email) - causing 15,000 sequential scans
3. Dev DB has 12 idle-in-transaction connections holding locks

Priority: Fix the locked queries in Production DB first - this is causing cascading delays."

---

**Q:** "What indexes should I create?"

**AI:** "I recommend creating these indexes in order of priority:

1. **Production DB**:
   - `CREATE INDEX CONCURRENTLY idx_orders_customer_id ON orders(customer_id)`
   - Impact: Saves $420/month, speeds up 80% of queries

2. **Staging DB**:
   - `CREATE INDEX CONCURRENTLY idx_users_email ON users(email)`
   - Impact: Saves $180/month, eliminates sequential scans

3. **Production DB**:
   - `CREATE INDEX CONCURRENTLY idx_events_created_at_brin ON events USING brin(created_at)`
   - Impact: 97% smaller than BTREE, perfect for time-series data"

---

**Q:** "Which database needs attention first?"

**AI:** "Production DB (health score: 62) needs immediate attention:

Critical Issues (3):
- Lock contention: 47 blocked queries
- Missing index on high-traffic table
- Slow query in payment processing (avg 2.3s)

Estimated Impact: Fixing these could improve response times by 78% and save $695/month.

Would you like me to generate the SQL to fix these issues?"

---

## 🔧 Advanced Configuration

### Environment Variables

**AI Configuration:**
```bash
# AI Provider
export DBO_AI_PROVIDER="openai"        # openai, anthropic, ollama
export DBO_AI_ENABLED=true

# API Keys
export OPENAI_API_KEY="sk-..."
export ANTHROPIC_API_KEY="sk-ant-..."

# Models
export DBO_AI_MODEL="gpt-4o-mini"      # or gpt-4o, claude-3-haiku-20240307, llama3.2

# Limits (to control costs)
export DBO_AI_MAX_QUERIES=5            # Max queries to analyze with AI
export DBO_AI_SLOW_THRESHOLD_MS=100.0  # Only analyze queries slower than this

# Ollama (local)
export OLLAMA_HOST="http://localhost:11434"
```

**HTTP Server Configuration:**
```bash
export HTTP_BIND_ADDRESS="0.0.0.0"
export HTTP_PORT=8080
export HTTP_CORS_ALLOWED_ORIGINS="*"
```

**eBPF Configuration:**
```bash
export EBPF_ENABLED=true                    # Requires Linux 5.8+
export EBPF_SAMPLING_RATE_HZ=100           # Sample rate
export EBPF_BUFFER_SIZE_KB=4096            # Ring buffer size
```

### Configuration File

Create `config.yaml`:
```yaml
ai:
  enabled: true
  provider: openai
  api_key: sk-...
  model: gpt-4o-mini
  max_queries_to_analyze: 5
  slow_query_threshold_ms: 100.0

http_server:
  bind_address: 0.0.0.0
  port: 8080
  cors_allowed_origins:
    - "*"

ebpf:
  enabled: true
  sampling_rate_hz: 100
  buffer_size_kb: 4096

persistence:
  backend: sqlite
  path: /var/lib/vaanyze/data.db
```

Start with config file:
```bash
./target/release/pg-query-advisor \
  --config config.yaml \
  --http-server
```

---

## 📊 API Endpoints Reference

### AI Endpoints

**POST /api/v1/ai/ask**
Ask AI a natural language question

Request:
```json
{
  "question": "Why is my database slow?",
  "context": "Fleet: 5 databases, 3 critical issues"
}
```

Response:
```json
{
  "status": "ok",
  "data": {
    "answer": "Based on your fleet...",
    "provider": "openai",
    "model": "gpt-4o-mini"
  }
}
```

**POST /api/v1/ai/optimize**
Optimize a query with AI

Request:
```json
{
  "query": "SELECT * FROM users WHERE...",
  "context": "Tables: users (10M rows), orders (50M rows)"
}
```

Response:
```json
{
  "status": "ok",
  "data": {
    "original": "SELECT * FROM...",
    "optimized": "SELECT users.id, users.email FROM...",
    "explanation": "Removed SELECT *, converted subquery to JOIN",
    "estimated_improvement_pct": 87.0,
    "provider": "openai"
  }
}
```

### Fleet Endpoints

**GET /api/v1/fleet**
List all database instances

**GET /api/v1/fleet/summary**
Fleet-wide summary

**POST /api/v1/fleet**
Register new instance

**POST /api/v1/fleet/:id/check**
Trigger health check

### Config Tuning Endpoints

**POST /api/v1/config/tune**
Generate optimal config

**GET /api/v1/config/validate**
Validate current config

**GET /api/v1/config/detect-workload**
Auto-detect workload type

### Lock Endpoints

**GET /api/v1/locks/summary**
Lock statistics summary

**GET /api/v1/locks/contention**
High contention tables

**GET /api/v1/locks/events**
Recent lock events

---

## 🎯 Best Practices

### 1. Cost Management

**Limit AI Usage:**
```bash
# Only analyze top 3 slowest queries
export DBO_AI_MAX_QUERIES=3

# Only analyze queries >500ms
export DBO_AI_SLOW_THRESHOLD_MS=500
```

**Use Cheaper Models:**
```bash
# GPT-4o-mini is 10x cheaper than GPT-4o
export DBO_AI_MODEL="gpt-4o-mini"

# Or use Ollama (free, local)
export DBO_AI_PROVIDER="ollama"
```

**Expected Costs (OpenAI):**
- Chat: ~$0.005 per message
- Query optimization: ~$0.01 per query
- 100 optimizations/day: ~$30/month

### 2. Security

**API Keys:**
```bash
# NEVER commit API keys to git
# Use environment variables or secret managers

# For production:
export OPENAI_API_KEY=$(aws secretsmanager get-secret-value --secret-id openai-key --query SecretString --output text)
```

**Database Credentials:**
```bash
# Use read-only user for monitoring
CREATE USER vaanyze_readonly WITH PASSWORD '...';
GRANT CONNECT ON DATABASE mydb TO vaanyze_readonly;
GRANT SELECT ON ALL TABLES IN SCHEMA public TO vaanyze_readonly;
GRANT SELECT ON pg_stat_statements TO vaanyze_readonly;
```

**CORS Configuration:**
```yaml
# Restrict origins in production
http_server:
  cors_allowed_origins:
    - "https://yourdomain.com"
```

### 3. Performance

**Enable eBPF (Linux only):**
```bash
# Provides 10x better lock analysis
export EBPF_ENABLED=true

# Requires CAP_BPF capability
sudo setcap cap_bpf,cap_perfmon=eip ./target/release/pg-query-advisor
```

**Optimize Refresh Intervals:**
```javascript
// In dashboard-enhanced.html, adjust polling intervals:

// Fleet data: every 5 seconds (default)
setInterval(fetchFleetData, 5000);

// Lock analysis: every 10 seconds (default)
setInterval(loadLockAnalysis, 10000);

// For large fleets, increase to 30 seconds:
setInterval(fetchFleetData, 30000);
```

---

## 🐛 Troubleshooting

### AI Not Working

**Error: "AI features not enabled"**
```bash
# Check environment variables
echo $OPENAI_API_KEY
echo $DBO_AI_ENABLED

# Set them:
export OPENAI_API_KEY="sk-..."
export DBO_AI_ENABLED=true

# Restart server
```

**Error: "AI service unavailable"**
- Check API key is valid
- Check network connectivity to api.openai.com
- Check rate limits (OpenAI: 500 requests/minute)

**Slow AI Responses:**
- Use `gpt-4o-mini` instead of `gpt-4o` (10x faster)
- Use Ollama locally (no network latency)
- Reduce query size by truncating context

### Dashboard Not Loading

**Blank page:**
```bash
# Check server is running
curl http://localhost:8080/api/v1/health

# Check browser console for errors (F12)
# Common issue: CORS - check HTTP server logs
```

**"Failed to fetch" errors:**
- Ensure HTTP server started with `--http-server` flag
- Check firewall rules
- Verify port 8080 is not blocked

### eBPF Not Working

**Error: "eBPF not supported"**
- Requires Linux kernel 5.8+
- Requires CAP_BPF capability
- Install kernel headers: `apt install linux-headers-$(uname -r)`

**Fallback to pg_stat:**
- Without eBPF, lock analysis still works via pg_stat_activity
- Less precise but still useful

---

## 📈 Monitoring & Metrics

### Prometheus Metrics

Enable metrics endpoint:
```bash
./target/release/pg-query-advisor \
  --http-server \
  --prometheus-metrics
```

Access metrics:
```
http://localhost:8080/metrics
```

**Available Metrics:**
```
# Fleet metrics
vaanyze_fleet_total_instances
vaanyze_fleet_healthy_count
vaanyze_fleet_critical_count
vaanyze_fleet_average_health_score

# AI metrics
vaanyze_ai_requests_total
vaanyze_ai_request_duration_seconds
vaanyze_ai_errors_total

# Lock metrics
vaanyze_locks_blocked_queries
vaanyze_locks_total_wait_time_ms
```

### Grafana Dashboard

Import the included dashboard:
```bash
# Coming soon: docs/grafana-dashboard.json
```

---

## 🎓 Example Workflows

### Workflow 1: Daily Database Health Check

**Morning Routine (5 minutes):**
1. Open dashboard → Fleet tab
2. Check critical issues count
3. If > 0:
   - Click database with lowest health score
   - Review AI recommendations
   - Apply high-priority fixes
4. Check AI chat for overnight alerts
5. Review cost savings banner

### Workflow 2: Optimizing a Slow Endpoint

**When users report slowness:**
1. Identify slow query from logs
2. Dashboard → Query Optimizer tab
3. Paste query
4. Click "Optimize with AI"
5. Review explanation
6. Copy optimized query
7. Test in staging
8. Deploy to production
9. Verify improvement in dashboard

### Workflow 3: New Database Setup

**Setting up optimal config:**
1. Register database in Fleet
2. Dashboard → Config Tuning tab
3. Select database
4. Let AI detect workload (or specify)
5. Click "Generate Optimal Configuration"
6. Download postgresql.conf
7. Apply to database
8. Restart PostgreSQL
9. Monitor health score improvement

### Workflow 4: Lock Contention Alert

**When locks spike:**
1. Dashboard → Lock Analysis tab
2. Identify blocking query in table
3. Options:
   - Click "Terminate" to kill query
   - Ask AI: "How do I prevent this lock?"
   - Apply recommended index
4. Monitor chart for improvement

---

## 🚀 Next Steps

1. **Start Simple**: Begin with OpenAI + one database
2. **Test AI**: Ask questions, optimize queries
3. **Scale Up**: Add more databases to fleet
4. **Automate**: Use API to integrate with CI/CD
5. **Monitor**: Set up Prometheus + Grafana
6. **Optimize**: Use AI recommendations to improve performance

---

## 💡 Tips & Tricks

**Keyboard Shortcuts:**
- `Ctrl/Cmd + K`: Focus AI chat
- `Esc`: Close modal
- `Tab`: Navigate between form fields

**Quick Prompts:**
- "Summarize the health of all databases"
- "What's the most expensive query?"
- "Generate migration script for all recommendations"
- "Explain health score calculation"

**Hidden Features:**
- Click health score → Show calculation breakdown
- Double-click instance card → Open in new tab
- Right-click recommendation → Copy SQL directly

---

## 📞 Support

**Issues?**
- GitHub Issues: https://github.com/anthropics/vaanyze/issues
- Documentation: `/docs`
- API Reference: `http://localhost:8080/api/v1` (when server running)

**Contributing:**
- See CONTRIBUTING.md
- Feature requests welcome!
- PRs accepted for bug fixes

---

**🎉 Enjoy your AI-powered database fleet management!**
