# Phase 0: Production-Ready Autonomous Postgres Co-Pilot - Implementation Plan

## Executive Summary

Transform `db-optimizer` from a CLI analysis tool into a **production-ready, autonomous, workload-aware Postgres co-pilot** that combines:
- **Deep observability**: pg_stat_statements + eBPF kernel metrics
- **Intelligent analysis**: Cost-based optimization + hardware/OS awareness
- **Safe automation**: Controlled, reversible, verifiable auto-apply with rollback
- **Developer UX**: CLI, HTTP API, and query rewriter assistant

## Current State Assessment

### ✅ Strong Foundation
- Working CLI with one-shot and agent modes
- DB stats collection via `pg_stat_statements`, tables, indexes
- Recommendation engine with cost estimation
- Query rewriter with 5 rule types (basic + enhanced versions)
- Auto-apply pipeline with baseline/verifier/safety filter
- eBPF module structure with metrics, correlation, daemon
- Agent mode with state persistence

### 🔧 Needs Enhancement
- **Config**: Minimal (only 3 fields), no mode controls or safety boundaries
- **eBPF Integration**: Module exists but not connected to main analysis flow
- **Persistence**: Agent state exists, but no historical query/recommendation tracking
- **Safety**: Basic risk assessment, lacks governance controls (protected resources, maintenance windows)
- **API**: No HTTP interface for programmatic access
- **Query Rewriter**: Exists but not exposed as standalone dev tool
- **Architecture**: Postgres-specific, not interface-driven for DB abstraction

---

## Implementation Plan (8 Major Phases)

### **Phase 1: Enhanced Configuration Layer** 
*Goal: Make this a configurable sidecar, not just a CLI tool*

**Files to Create/Modify:**
- `src/config.rs` → Comprehensive config with modes, safety, eBPF toggles
- `config.example.yml` → Example configuration file
- `src/config/` → New submodules for config validation and parsing

**New Structures:**
```rust
pub enum OperationMode {
    ObserveOnly,      // Read-only analysis
    SafeAuto,         // Auto-apply safe operations only
    Aggressive,       // More aggressive (staging/opt-in)
}

pub struct SafetyConfig {
    pub protected_schemas: Vec<String>,
    pub protected_tables: Vec<String>,
    pub protected_columns: HashMap<String, Vec<String>>,
    pub max_index_size_mb: f64,
    pub max_ddl_lock_time_secs: u64,
}

pub struct MaintenanceWindow {
    pub enabled: bool,
    pub allowed_days: Vec<Weekday>,
    pub allowed_hours_utc: RangeInclusive<u8>,
}

pub struct EbpfConfig {
    pub enabled: bool,
    pub sampling_interval_ms: u64,
    pub collect_cpu_metrics: bool,
    pub collect_io_metrics: bool,
    pub collect_memory_metrics: bool,
}
```

**Changes:**
- Extend `AutoApplyConfig` to reference `OperationMode` and `SafetyConfig`
- Add config file loading (YAML/TOML via serde)
- Support env var overrides for cloud deployments
- Validate config on load with clear error messages

---

### **Phase 2: Core Architecture - Trait-Based Abstraction**
*Goal: Make core concepts DB-agnostic while keeping Postgres as first implementation*

**Files to Create:**
- `src/core/mod.rs` → Core traits and interfaces
- `src/core/metrics.rs` → `MetricsCollector` trait
- `src/core/analyzer.rs` → `Analyzer` trait
- `src/core/recommendations.rs` → `RecommendationEngine` trait
- `src/core/orchestrator.rs` → `AutoApplyOrchestrator` trait

**New Traits:**
```rust
#[async_trait]
pub trait MetricsCollector: Send + Sync {
    async fn collect_query_stats(&self, limit: i64) -> Result<Vec<QueryStats>>;
    async fn collect_table_info(&self) -> Result<Vec<TableInfo>>;
    async fn collect_index_info(&self) -> Result<Vec<IndexInfo>>;
    async fn get_database_info(&self) -> Result<(String, String)>;
}

pub trait Analyzer: Send + Sync {
    fn analyze(&self, 
        queries: &[QueryStats], 
        tables: &[TableInfo], 
        indexes: &[IndexInfo],
        system_metrics: Option<&UnifiedMetrics>,
    ) -> Vec<Recommendation>;
}
```

**Changes:**
- Refactor `DatabaseCollector` → `PostgresMetricsCollector` implementing trait
- Keep existing public API stable
- Add factory pattern: `MetricsCollector::new_postgres(pool)`
- Document extension points for MySQL/CockroachDB future support

---

### **Phase 3: eBPF + DB Stats Unified Correlation**
*Goal: Make eBPF the differentiator - correlate kernel + DB metrics*

**Files to Create:**
- `src/ebpf/unified_metrics.rs` → `UnifiedQueryMetrics` structure
- `src/ebpf/collector_bridge.rs` → Bridge eBPF daemon to main collector

**New Structure:**
```rust
pub struct UnifiedQueryMetrics {
    // DB-side (from pg_stat_statements)
    pub query_id: i64,
    pub query_text: String,
    pub db_calls: i64,
    pub db_total_time_ms: f64,
    pub db_mean_time_ms: f64,
    
    // eBPF-side (from kernel tracing)
    pub pid: Option<u32>,
    pub cpu_time_us: u64,
    pub io_wait_time_us: u64,
    pub page_cache_misses: u32,
    pub context_switches: u32,
    pub bytes_read: u64,
    pub bytes_written: u64,
    
    // Correlation metadata
    pub confidence_score: f64,
    pub sample_count: u32,
}

pub struct RootCauseInsight {
    pub category: InsightCategory, // IO_BOUND, CPU_BOUND, LOCK_CONTENTION
    pub description: String,
    pub evidence: Vec<String>,
    pub severity: f64, // 0.0-1.0
}
```

**Changes:**
- Extend `Analyzer` to consume `UnifiedQueryMetrics`
- Add correlation logic in `ebpf/correlator.rs` (already exists) to map:
  - Postgres PIDs → query IDs
  - Time ranges → query execution windows
- Generate root-cause insights: "Query X slow due to high IO wait (85%) + page cache misses"
- Handle graceful degradation if eBPF unavailable (permissions, kernel version)

---

### **Phase 4: Persistence & History Layer**
*Goal: Track trends, wins, regressions over time*

**Files to Create:**
- `src/persistence/mod.rs` → Persistence trait + factory
- `src/persistence/sqlite.rs` → SQLite backend (embedded)
- `src/persistence/postgres.rs` → Postgres backend (use same DB)
- `src/persistence/schema.sql` → Schema migrations

**New Tables:**
```sql
-- Query performance history
CREATE TABLE query_samples (
    id INTEGER PRIMARY KEY,
    captured_at TIMESTAMP,
    query_id BIGINT,
    query_text TEXT,
    mean_exec_time_ms REAL,
    p95_exec_time_ms REAL,
    calls_per_min REAL,
    -- eBPF metrics
    cpu_time_us BIGINT,
    io_wait_time_us BIGINT,
    UNIQUE(query_id, captured_at)
);

-- Recommendations history
CREATE TABLE recommendations (
    id TEXT PRIMARY KEY,
    created_at TIMESTAMP,
    issue_type TEXT,
    severity TEXT,
    estimated_savings_usd REAL,
    status TEXT, -- PENDING, APPLIED, REJECTED, ROLLED_BACK
    applied_at TIMESTAMP,
    rollback_at TIMESTAMP
);

-- Change events
CREATE TABLE change_events (
    id INTEGER PRIMARY KEY,
    recommendation_id TEXT,
    event_type TEXT, -- PROPOSED, APPLIED, VERIFIED, ROLLED_BACK
    timestamp TIMESTAMP,
    metadata JSON
);

-- Baseline snapshots
CREATE TABLE baselines (
    id INTEGER PRIMARY KEY,
    recommendation_id TEXT,
    baseline_type TEXT, -- PRE_APPLY, POST_APPLY
    captured_at TIMESTAMP,
    metrics JSON
);
```

**Changes:**
- Add `--persistence-backend` CLI flag: `sqlite` (default), `postgres`, `none`
- Store query samples on each agent cycle
- Record all auto-apply events (proposed → applied → verified → rolled back)
- Enable queries like "top 10 wins this month" or "regressions after last deploy"

---

### **Phase 5: Hardened Auto-Apply Safety & Governance**
*Goal: Make auto-apply safe enough for production*

**Files to Modify:**
- `src/auto_apply/safety_filter.rs` → Enhanced with config-driven rules
- `src/auto_apply/governance.rs` → NEW: Governance checks

**New Logic:**
```rust
impl SafetyFilter {
    fn check_governance(&self, rec: &Recommendation) -> GovernanceResult {
        // 1. Check protected resources
        if self.is_protected_resource(&rec) {
            return GovernanceResult::Blocked("Protected resource");
        }
        
        // 2. Check maintenance window
        if !self.in_maintenance_window() {
            return GovernanceResult::Deferred("Outside maintenance window");
        }
        
        // 3. Check operation mode
        match self.config.operation_mode {
            ObserveOnly => GovernanceResult::Blocked("Observe-only mode"),
            SafeAuto => {
                if rec.risk_level > RiskLevel::Low {
                    GovernanceResult::RequiresApproval
                } else {
                    GovernanceResult::Allowed
                }
            }
            Aggressive => {
                if rec.risk_level >= RiskLevel::Critical {
                    GovernanceResult::RequiresApproval
                } else {
                    GovernanceResult::Allowed
                }
            }
        }
    }
}
```

**Enhanced Baseline/Verifier:**
- Compare eBPF metrics pre/post (CPU, IO, not just query time)
- Support custom success criteria (e.g., "p99 must improve by 20%")
- Add "soak period" before declaring success (prevent false positives)
- Better rollback logic: check dependencies before DROP INDEX

**Changes:**
- Never touch primary keys or unique constraints unless explicitly allowlisted
- Enforce max index size limits
- Estimate DDL duration based on table size; reject if > threshold
- Log all governance decisions to persistence layer

---

### **Phase 6: HTTP/JSON API**
*Goal: Expose programmatic interface for web UI, monitoring, integrations*

**Files to Create:**
- `src/http_server.rs` → Main HTTP server (Axum)
- `src/http/handlers.rs` → Request handlers
- `src/http/models.rs` → API request/response models

**Endpoints:**
```
GET  /health                          → Health check
GET  /metrics                         → Prometheus metrics
GET  /api/v1/queries/slow             → Slow queries (paginated)
GET  /api/v1/queries/:id              → Query detail + history
GET  /api/v1/recommendations          → Current recommendations
GET  /api/v1/recommendations/:id      → Recommendation detail
POST /api/v1/recommendations/:id/apply → Manually trigger apply
GET  /api/v1/changes                  → Change history
GET  /api/v1/changes/:id              → Change detail
GET  /api/v1/stats/summary            → Dashboard summary
GET  /api/v1/config                   → Current config (redacted)
```

**Response Examples:**
```json
{
  "queries": [
    {
      "query_id": 12345,
      "query": "SELECT * FROM users WHERE ...",
      "mean_time_ms": 450.2,
      "calls_per_min": 1200,
      "root_cause": {
        "category": "IO_BOUND",
        "description": "High IO wait (82%) with page cache misses",
        "severity": 0.85
      }
    }
  ],
  "total": 42,
  "page": 1
}
```

**Changes:**
- Add `--http-server` flag to enable API mode
- Add `--http-bind` for address (default: `127.0.0.1:9090`)
- Use middleware for auth (optional bearer token)
- CORS support for web UI development
- OpenAPI/Swagger spec generation

---

### **Phase 7: Query Rewriter as Dev Tool**
*Goal: Turn rewriter into CI/IDE/ORM integration point*

**Files to Modify:**
- `src/main.rs` → Add `rewrite-sql` subcommand
- `src/query_rewriter/mod.rs` → Enhanced output format

**New CLI Subcommand:**
```bash
# Interactive mode
db-optimizer rewrite-sql --input query.sql --format json

# Pipe mode
cat query.sql | db-optimizer rewrite-sql --format markdown

# CI mode (exit code 1 if optimizations available)
db-optimizer rewrite-sql --input query.sql --strict
```

**Output Format:**
```json
{
  "status": "optimized",
  "original_query": "SELECT * FROM users",
  "optimized_query": "SELECT id, name, email FROM users LIMIT 1000",
  "applied_rules": [
    {
      "name": "SelectStarRemover",
      "description": "Replaced * with explicit columns",
      "improvement_pct": 15.0
    },
    {
      "name": "LimitAdder", 
      "description": "Added LIMIT to unbounded query",
      "improvement_pct": 50.0
    }
  ],
  "total_improvement_pct": 57.5,
  "reasoning": [
    "SELECT * fetches unnecessary columns (network overhead)",
    "Unbounded queries can cause timeouts on large tables"
  ]
}
```

**Changes:**
- Standardize `RewriteRule` interface to return structured output
- Add `--db-url` flag to enable schema-aware SELECT * removal
- Support batch mode: rewrite all queries in a directory
- Generate GitHub Actions workflow example for CI integration

---

### **Phase 8: Packaging, Deployment, Testing**
*Goal: Make it trivial to deploy and trust*

**Files to Create:**

**Docker:**
- `Dockerfile` → Multi-stage build (small runtime image)
- `docker-compose.yml` → Complete example with Postgres + agent
- `.dockerignore`

**Kubernetes:**
- `deploy/k8s/deployment.yaml` → Deployment manifest
- `deploy/k8s/configmap.yaml` → Config example
- `deploy/k8s/rbac.yaml` → ServiceAccount (if using eBPF)
- `deploy/helm/` → Basic Helm chart scaffold

**CI/CD:**
- `.github/workflows/ci.yml` → Test, lint, build
- `.github/workflows/release.yml` → Release automation

**Tests:**
- `tests/unit/` → Unit tests for all major modules
- `tests/integration/` → Integration tests with testcontainers
- `tests/e2e/` → End-to-end scenarios

**Documentation:**
- `README.md` → Update with "Getting Started" section
- `docs/ARCHITECTURE.md` → Architecture overview
- `docs/DEPLOYMENT.md` → Deployment guide
- `docs/API.md` → HTTP API reference
- `docs/CONFIGURATION.md` → Config reference

**CI Checks:**
```yaml
- cargo fmt --check
- cargo clippy -- -D warnings
- cargo test --all-features
- cargo build --release
- Integration tests with Postgres 14, 15, 16
```

---

## File-by-File Changes Summary

### New Files (23 files)
```
config.example.yml
PHASE_0_PLAN.md
src/core/mod.rs
src/core/metrics.rs
src/core/analyzer.rs
src/core/recommendations.rs
src/core/orchestrator.rs
src/ebpf/unified_metrics.rs
src/ebpf/collector_bridge.rs
src/persistence/mod.rs
src/persistence/sqlite.rs
src/persistence/postgres.rs
src/persistence/schema.sql
src/auto_apply/governance.rs
src/http_server.rs
src/http/handlers.rs
src/http/models.rs
Dockerfile
docker-compose.yml
deploy/k8s/deployment.yaml
deploy/helm/Chart.yaml
.github/workflows/ci.yml
docs/ARCHITECTURE.md
docs/API.md
```

### Modified Files (11 files)
```
src/config.rs          → Comprehensive config with modes, safety, eBPF
src/main.rs           → Add rewrite-sql subcommand, http-server mode
src/collector.rs      → Implement MetricsCollector trait
src/analyzer.rs       → Consume UnifiedQueryMetrics
src/models.rs         → Add UnifiedQueryMetrics, RootCauseInsight
src/ebpf/mod.rs       → Export unified metrics types
src/auto_apply/safety_filter.rs → Add governance checks
src/auto_apply/verifier.rs → Compare eBPF metrics
src/query_rewriter/mod.rs → Enhanced output format
Cargo.toml           → Add axum, tower, rusqlite, tokio dependencies
README.md            → Getting Started section
```

---

## Product Vision Mapping

| Requirement | Implementation |
|------------|----------------|
| **Observe** | ✅ pg_stats + eBPF unified metrics + persistence history |
| **Analyze** | ✅ Cost-based + eBPF root-cause insights + HW/OS awareness |
| **Recommend** | ✅ Indexes, rewrites, config tuning via existing engine |
| **Auto-Apply** | ✅ Safe modes + governance + verification + rollback |
| **Explain** | ✅ CLI formats + HTTP API + dev-friendly query rewriter |

---

## Implementation Order (Priority)

1. **Phase 1** (Config) - Foundation for everything else
2. **Phase 4** (Persistence) - Enable history tracking early
3. **Phase 3** (eBPF correlation) - Core differentiator
4. **Phase 5** (Safety hardening) - Must-have for production
5. **Phase 2** (Architecture traits) - Can be incremental
6. **Phase 6** (HTTP API) - High value, parallelizable
7. **Phase 7** (Query rewriter tool) - Independent feature
8. **Phase 8** (Packaging/tests) - Continuous throughout

---

## Risk Mitigation

### Breaking Changes
- Keep existing CLI interface working
- Add new features as opt-in flags
- Version HTTP API (`/api/v1/`)

### eBPF Availability
- Graceful degradation if eBPF fails to load
- Clear error messages: "eBPF requires root or CAP_BPF"
- Fall back to DB-only analysis

### Performance Impact
- eBPF sampling configurable (default: 100ms)
- Persistence writes async/batched
- HTTP server on separate tokio runtime if needed

### Testing Strategy
- Unit tests for business logic (no DB required)
- Integration tests with testcontainers
- E2E tests with real Postgres + sample workload

---

## Success Criteria

✅ **Week 1-2**: Config + Persistence functional, tests passing
✅ **Week 3**: eBPF correlation working, root-cause insights generated
✅ **Week 4**: Safety hardening complete, governance enforced
✅ **Week 5**: HTTP API live, Swagger docs generated
✅ **Week 6**: Query rewriter CLI + CI examples
✅ **Week 7**: Docker + K8s deployable, README updated
✅ **Week 8**: CI passing, integration tests green, ready for alpha users

---

## Next Steps

After approval of this plan, I will:
1. Start with **Phase 1** (Enhanced Config) - create comprehensive config structures
2. Move to **Phase 4** (Persistence) - add SQLite backend first
3. Then **Phase 3** (eBPF correlation) - unify metrics
4. Continue through remaining phases incrementally

Each phase will be implemented with:
- Complete working code
- Unit tests where applicable
- Updated documentation
- Backwards compatibility maintained

**Ready to proceed?** 🚀
