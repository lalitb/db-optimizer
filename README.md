# Vaanyze - PostgreSQL Index Optimization with EXPLAIN Validation

A tool that analyzes PostgreSQL query statistics and generates index recommendations. Recommendations are validated with EXPLAIN before/after comparison and only applied if proof shows the query planner will use them.

**Default behavior:** Read-only analysis. No database modifications unless explicitly enabled.

---

## Safety, Scope & Guarantees

### What Vaanyze Enforces by Default

| Guarantee | Enforcement |
|-----------|-------------|
| **Read-only by default** | No CREATE/DROP INDEX unless `--auto-apply` flag AND mode ≠ observe_only |
| **Forced read-only on managed DBs** | RDS, Aurora, Cloud SQL, Azure, Heroku automatically blocked from apply |
| **Kill switch** | `POST /api/v1/system/kill-switch` immediately stops all apply/rollback |
| **EXPLAIN validation required** | Index only applied if planner proves it will be used |
| **CONCURRENTLY always** | All CREATE/DROP INDEX use CONCURRENTLY (no table locks) |
| **Verification window** | After apply, monitors for regression; rolls back if detected |
| **Audit logging** | All operations logged (in-memory by default) |

### What Requires Explicit Opt-In

| Feature | How to Enable | Notes |
|---------|---------------|-------|
| Auto-apply | `--auto-apply` flag or API mode change | Requires mode ≠ observe_only |
| AI recommendations | `--ai` flag + API key | External API calls to OpenAI/Claude/Ollama |
| eBPF profiling | `--enable-ebpf` | Linux only, requires CAP_BPF |
| Agent mode | `agent` subcommand | Continuous background monitoring |

### What Is Experimental (No Production Guarantees)

| Feature | Status | Warning |
|---------|--------|---------|
| eBPF lock contention | Experimental | Linux kernel 5.8+ required; gracefully degrades |
| CloudNativePG plugin | Beta v0.1.0 | Limited production testing |
| Web dashboard | Development | Not security-audited; intended for local use |
| Fleet management | Development | In-memory state only; lost on restart |

### What Vaanyze Cannot Do

- Modify databases on managed services (RDS, Cloud SQL, etc.) — **enforced block**
- Guarantee query improvement — EXPLAIN estimates are not runtime measurements
- Work reliably with connection poolers in transaction mode — PID correlation breaks
- Persist state across restarts — state machine is in-memory (roadmap item)
- Replace a DBA — complex schema decisions require human judgment

---

## Core MVP Features (Enforced, Pilot-Ready)

- 🚀 **Fast Analysis** — Analyzes databases in seconds using pg_stat_statements
- 🧠 **Data-Driven** — Uses PostgreSQL's own statistics (pg_stats) for recommendations
- 🔒 **Safe Defaults** — Read-only by default; forced read-only on managed DBs
- 🎯 **EXPLAIN Validation** — Proves index will be used before applying
- ↩️ **Automatic Rollback** — Reverts changes if verification detects regression
- 🛑 **Kill Switch** — Emergency stop for all apply/rollback operations
- 💰 **Cost Estimation** — Shows potential savings (estimates, not guarantees)

## Advanced Features (Opt-In)

- 🤖 **AI Integration** — Optional LLM support (OpenAI, Claude, Ollama) with `--ai` flag
- 🔄 **Auto-Apply** — Apply validated recommendations with `--auto-apply` flag
- 📊 **Agent Mode** — Continuous monitoring with `agent` subcommand
- ✏️ **Query Rewriting** — Suggestions for query optimization with `--enable-rewriter`
- 📊 **Advanced Index Types** — BRIN, GIN, GIST, HASH with `--advanced-indexes`

## Experimental Features (Contributor-Focused)

- 🔬 **eBPF Profiling** — Kernel-level I/O and lock tracking (Linux only)
- ☸️ **CloudNativePG Plugin** — Kubernetes integration (beta)
- 🖥️ **Web Dashboard** — Visual interface (development, not security-audited)

## What It Detects

### Index Recommendations
- **BTREE** — General purpose indexes (equality + range queries)
- **HASH** — High cardinality columns with equality-only queries (with `--advanced-indexes`)
- **GIN** — JSONB columns, arrays, full-text search (with `--advanced-indexes`)
- **GIST** — Geometric/spatial data (with `--advanced-indexes`)
- **BRIN** — Sequential data on large tables (with `--advanced-indexes`)
- **Partial Indexes** — For skewed distributions (with `--advanced-indexes`)

### Query Issues
- **Slow queries** that need optimization
- **Missing indexes** on frequently queried columns
- **Unused indexes** wasting storage

### Auto-Apply (Opt-In with `--auto-apply`)

> ⚠️ **Requires explicit opt-in.** Disabled by default. Blocked on managed databases.

- EXPLAIN validation before apply
- Baseline capture before changes
- Verification window after apply
- Automatic rollback on regression
- Slack notifications (optional)

---

## Quick Start

### Option 1: CLI Analysis (Recommended for First Use)

```bash
# Build
cargo build --release

# Analyze database (read-only, safe)
./target/release/db-optimizer analyze -d postgresql://user:pass@localhost/mydb

# This will:
# - Connect to your database
# - Query pg_stat_statements for slow queries
# - Generate index recommendations
# - Output to terminal (no changes made)
```

### Option 2: HTTP Server Mode

```bash
# Start server (read-only by default)
./target/release/db-optimizer serve --port 9090 -d postgresql://user:pass@localhost/mydb

# Check health
curl http://localhost:9090/api/v1/health
```

### Option 3: Web Dashboard (Development)

> ⚠️ **Development feature.** Not security-audited. For local use only.

```bash
# Start HTTP server
./target/release/db-optimizer serve --port 8080 -d postgresql://user:pass@localhost/mydb

# Open browser
# http://localhost:8080/dashboard.html
```

Optional AI features require API keys:
```bash
export OPENAI_API_KEY="sk-..."        # or
export ANTHROPIC_API_KEY="sk-ant-..." # or
ollama pull llama3.2                   # local, no key needed
export DBO_AI_PROVIDER="ollama"
export DBO_AI_ENABLED=true
```

### Prerequisites

Your PostgreSQL database must have `pg_stat_statements` extension enabled:

```sql
-- Run as superuser
CREATE EXTENSION IF NOT EXISTS pg_stat_statements;

-- Recommended: Update statistics
ANALYZE;
```

---

## CLI Usage

```bash
# Basic analysis (read-only, safe)
./target/release/db-optimizer analyze -d $DATABASE_URL

# With advanced index types (opt-in)
./target/release/db-optimizer analyze -d $DATABASE_URL --advanced-indexes

# Enable query rewriting suggestions (opt-in)
./target/release/db-optimizer analyze -d $DATABASE_URL --enable-rewriter

# Export to JSON
./target/release/db-optimizer analyze -d $DATABASE_URL --format json --output report.json

# HTTP Server mode
./target/release/db-optimizer serve --port 9090 -d $DATABASE_URL
```

### Auto-Apply (Opt-In)

> ⚠️ **Destructive operations.** Only works on self-managed PostgreSQL. Blocked on RDS/Cloud SQL/etc.

```bash
# Auto-apply with low-risk threshold
./target/release/db-optimizer analyze -d $DATABASE_URL --auto-apply --auto-approve low

# With Slack notifications
./target/release/db-optimizer analyze -d $DATABASE_URL --auto-apply --slack-webhook $WEBHOOK
```

### Agent Mode (Opt-In)

> ⚠️ **Continuous monitoring.** Runs indefinitely. Does not auto-apply unless explicitly configured.

```bash
./target/release/db-optimizer agent \
  --database-url $DATABASE_URL \
  --check-interval 3600 \
  --slack-webhook $SLACK_WEBHOOK
```

---

## Experimental Features

### eBPF Profiling (Linux Only)

> ⚠️ **Experimental.** Requires Linux kernel 5.8+, CAP_BPF capability. Gracefully degrades when unavailable.

```bash
# Requirements
uname -r  # Should be >= 5.8

# Grant capabilities
sudo setcap cap_bpf,cap_perfmon=eip ./target/release/db-optimizer

# Build eBPF programs
make -C bpf

# Start with eBPF enabled
./target/release/db-optimizer serve --port 8080 -d $DATABASE_URL --enable-ebpf
```

Without eBPF: Lock analysis falls back to `pg_stat_activity` (less precise but still useful).

### CloudNativePG Plugin (Beta)

> ⚠️ **Beta v0.1.0.** Limited production testing. Use with caution.

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

---

## API Reference

### Core Endpoints

```bash
# Health check (shows operation mode and guard status)
curl http://localhost:9090/api/v1/health

# Environment detection
curl http://localhost:9090/api/v1/system/environment

# Kill switch (emergency stop)
curl -X POST http://localhost:9090/api/v1/system/kill-switch \
  -H "Content-Type: application/json" \
  -d '{"enabled": true, "reason": "Emergency"}'

# Set operation mode
curl -X POST http://localhost:9090/api/v1/system/mode \
  -H "Content-Type: application/json" \
  -d '{"mode": "observe_only"}'  # or "safe_auto", "aggressive"
```

### Validation & Apply Endpoints

```bash
# Validate an index recommendation (EXPLAIN proof)
curl -X POST http://localhost:9090/api/v1/index/validate \
  -H "Content-Type: application/json" \
  -d '{"query": "SELECT ...", "table_name": "users", "columns": ["email"]}'

# Apply a validated recommendation (requires mode != observe_only)
curl -X POST http://localhost:9090/api/v1/index/apply \
  -H "Content-Type: application/json" \
  -d '{"recommendation_id": "rec_123"}'

# Rollback an applied index
curl -X POST http://localhost:9090/api/v1/index/rollback \
  -H "Content-Type: application/json" \
  -d '{"recommendation_id": "rec_123"}'

# View audit log
curl http://localhost:9090/api/v1/index/audit
```

### AI Endpoints (Opt-In)

> Requires AI API key configuration.

```bash
# Ask a question
curl -X POST http://localhost:9090/api/v1/ai/ask \
  -H "Content-Type: application/json" \
  -d '{"question": "Why is my database slow?"}'

# Optimize a query
curl -X POST http://localhost:9090/api/v1/ai/optimize \
  -H "Content-Type: application/json" \
  -d '{"query": "SELECT * FROM users WHERE..."}'
```

**Supported LLM Providers:**

| Provider | Models | API Key Env Var |
|----------|--------|-----------------|
| OpenAI | gpt-4-turbo-preview, gpt-4, gpt-3.5-turbo | `OPENAI_API_KEY` |
| Anthropic | claude-3-sonnet, claude-3-opus | `ANTHROPIC_API_KEY` |
| Ollama | llama2, mistral, codellama (local) | None (local) |

---

## How It Works

### Analysis Pipeline

1. **Collection** — Gathers data from PostgreSQL system views:
   - `pg_stat_statements` — Query statistics
   - `pg_stats` — Column distribution data
   - `pg_stat_user_indexes` — Index usage

2. **Analysis** — Identifies optimization opportunities:
   - Missing indexes on frequently queried columns
   - Unused indexes wasting storage
   - Query patterns that could benefit from specific index types

3. **Validation** — EXPLAIN-based proof (when auto-apply enabled):
   - Creates candidate index with `_vaanyze_candidate_` prefix
   - Runs EXPLAIN before/after
   - Validates query planner uses the index
   - Measures cost improvement
   - Drops candidate regardless of outcome

4. **Apply** (opt-in) — Safe execution:
   - Uses `CREATE INDEX CONCURRENTLY` (no table locks)
   - Verification window monitors for regression
   - Automatic rollback if performance degrades

### State Machine

```
NEW → VALIDATED → APPLIED → VERIFYING → STABLE
         ↓           ↓          ↓
       FAILED     FAILED    ROLLED_BACK
```

---

## Advanced Features (Opt-In)

### Advanced Index Types (`--advanced-indexes`)

Selects optimal index type based on data characteristics:

```bash
./target/release/db-optimizer analyze -d $DATABASE_URL --advanced-indexes
```

| Index Type | Use Case | Flag Required |
|------------|----------|---------------|
| BTREE | General purpose (default) | None |
| BRIN | Sequential data (timestamps, auto-increment) | `--advanced-indexes` |
| GIN | JSONB, arrays, full-text search | `--advanced-indexes` |
| GIST | Geometric/spatial data | `--advanced-indexes` |
| HASH | High cardinality equality-only | `--advanced-indexes` |
| Partial | Skewed distributions | `--advanced-indexes` |

### Query Rewriting (`--enable-rewriter`)

Suggests query structure optimizations:

```bash
./target/release/db-optimizer analyze -d $DATABASE_URL --enable-rewriter
```

Transformations suggested:
- `SELECT *` → Explicit column list
- Subqueries → JOINs (when beneficial)
- Predicate pushdown
- Add LIMIT to unbounded queries

---

## Limitations

### Environment Constraints

| Constraint | Impact |
|------------|--------|
| **Managed databases (RDS, Cloud SQL, etc.)** | Auto-apply blocked. Use observe_only mode. |
| **Connection poolers (transaction mode)** | PID correlation unreliable. Run against PostgreSQL directly. |
| **PostgreSQL < 12** | Limited feature support |
| **Without pg_stat_statements** | Cannot analyze queries |

### eBPF Constraints

| Constraint | Impact |
|------------|--------|
| **Linux only** | eBPF features disabled on macOS/Windows |
| **Kernel < 5.8** | Limited eBPF features |
| **No CAP_BPF** | Graceful degradation to pg_stat_activity |
| **Stripped binaries** | uprobe-based lock tracing unavailable |

### What Vaanyze Cannot Guarantee

- **Query improvement** — EXPLAIN estimates are not runtime measurements
- **State persistence** — In-memory by default; lost on restart (P1 roadmap)
- **Audit persistence** — In-memory by default (configurable)
- **Connection pooler support** — Transaction-mode poolers break PID correlation

See [docs/LIMITATIONS.md](docs/LIMITATIONS.md) for complete details.

---

## Roadmap

### ✅ Implemented (Enforced)

- Index recommendations from pg_stat_statements
- EXPLAIN-based validation before apply
- Safe apply with CONCURRENTLY + verification + rollback
- Kill switch and mode enforcement
- Environment detection and forced read-only on managed DBs
- Audit logging

### ✅ Implemented (Opt-In)

- Advanced index types (BRIN, GIN, GIST, HASH, Partial)
- Query rewriting suggestions
- Auto-apply mode
- Agent mode (continuous monitoring)
- AI/LLM integration (OpenAI, Claude, Ollama)

### 🧪 Experimental

- eBPF kernel-level profiling
- CloudNativePG plugin (beta)
- Web dashboard
- Fleet management

### 📅 Planned (Not Implemented)

- State persistence across restarts (P1)
- Audit log persistence to disk (P1)
- Background verification loop (P1)
- Team collaboration and RBAC
- Grafana/Prometheus dashboards
- Multi-tenant support

---

## Development

```bash
# Build
cargo build --release

# Run tests
cargo test

# Format
cargo fmt

# Lint
cargo clippy
```

## Contributing

Contributions welcome. Areas where help is needed:

- Additional index type heuristics
- Query rewriting rules
- Performance benchmarks
- Documentation improvements
- Integration tests

Please open an issue first to discuss changes.

## License

MIT

## Documentation

- [SAFETY.md](docs/SAFETY.md) — Safety model and rollback behavior
- [LIMITATIONS.md](docs/LIMITATIONS.md) — Known limitations and constraints
- [MVP_GAP_AUDIT.md](docs/MVP_GAP_AUDIT.md) — Current status and gaps
- [pilot-guide.md](docs/pilot-guide.md) — Running a pilot
- [CNPG_QUICK_START.md](CNPG_QUICK_START.md) — CloudNativePG integration
