# db-optimizer Documentation

**PostgreSQL Performance Optimization Tool**

db-optimizer is a comprehensive PostgreSQL performance optimization tool that combines query analysis, configuration tuning, lock contention tracking, and automatic remediation in a single package.

---

## 🚀 Quick Start

```bash
# Install
cargo build --release

# Analyze your database
db-optimizer analyze -d $DATABASE_URL

# Tune PostgreSQL configuration
db-optimizer config-tune -d $DATABASE_URL

# Validate current config
db-optimizer config-validate -d $DATABASE_URL

# Start HTTP API server
db-optimizer serve --port 8080
```

---

## 📋 Table of Contents

- [Features](#features)
- [Architecture](#architecture)
- [CLI Commands](#cli-commands)
- [HTTP API](#http-api)
- [Configuration](#configuration)
- [eBPF Integration](#ebpf-integration)
- [Deployment](#deployment)
- [Development](#development)

---

## ✨ Features

### 1. Query Analysis & Recommendations
- Analyzes pg_stat_statements for slow queries
- Generates index recommendations
- Identifies missing indexes, unused indexes, duplicate indexes
- Query rewriting suggestions
- Cost estimation and savings calculation

### 2. PostgreSQL Configuration Tuning
- **Auto-detect workload type** (Desktop, Web, OLTP, Data Warehouse, Mixed)
- **PGTune-inspired calculations** for optimal settings
- Tunes 15+ parameters:
  - Memory: shared_buffers, effective_cache_size, work_mem, maintenance_work_mem
  - Connections: max_connections, max_worker_processes, max_parallel_workers
  - Query Planning: random_page_cost, effective_io_concurrency, default_statistics_target
  - WAL & Checkpointing: checkpoint_completion_target, wal_buffers, min/max_wal_size
  - Logging: log_min_duration_statement
- **Safety validators** prevent OOM and invalid configurations
- Multiple output formats (terminal, JSON, markdown)

### 3. Lock Contention Tracking (eBPF)
- Real-time lock contention monitoring via eBPF
- Identifies high-contention tables
- Tracks lock wait times and frequencies
- No PostgreSQL instrumentation overhead

### 4. Auto-Apply with Safety Guardrails
- Automatic application of recommendations
- Baseline capture and verification
- Automatic rollback on regression
- Configurable risk thresholds
- Slack/webhook notifications

### 5. HTTP API
- RESTful API for integration
- Query analysis endpoints
- Configuration tuning endpoints
- Lock monitoring endpoints
- Prometheus metrics export

### 6. Query Rewriter
- Predicate pushdown
- SELECT * removal
- Subquery flattening
- Redundant join removal
- LIMIT addition for safety
- Cost-based optimization

---

## 🏗️ Architecture

```
┌─────────────────────────────────────────────────────────┐
│                    db-optimizer                         │
├─────────────────────────────────────────────────────────┤
│  CLI Interface        │  HTTP API Server               │
├───────────────────────┼────────────────────────────────┤
│  Query Analyzer       │  Config Tuner                   │
│  - pg_stat_statements │  - Workload Detector            │
│  - Cost Estimator     │  - Config Calculator            │
│  - Index Recommender  │  - Config Validator             │
├───────────────────────┼────────────────────────────────┤
│  Query Rewriter       │  eBPF Integration               │
│  - Predicate Pushdown │  - Lock Contention Tracking     │
│  - Subquery Flatten   │  - Process Metrics              │
│  - Join Optimizer     │  - Real-time Monitoring         │
├───────────────────────┴────────────────────────────────┤
│  Auto-Apply Orchestrator                                │
│  - Safety Filter      │  - Verifier                     │
│  - Baseline Capture   │  - Executor                     │
│  - Rollback Engine    │  - Notifier                     │
├─────────────────────────────────────────────────────────┤
│  PostgreSQL Database                                    │
└─────────────────────────────────────────────────────────┘
```

**Key Components:**
- **Adapters Layer**: PostgreSQL collector, query analyzer, recommendation engine
- **Core Engine**: Analysis, cost estimation, recommendation generation
- **Config Tuner**: Workload detection, config calculation, validation
- **Query Rewriter**: SQL optimization rules
- **Auto-Apply**: Safe automated remediation pipeline
- **eBPF**: Kernel-level monitoring (optional)
- **Persistence**: SQLite for history and state
- **HTTP API**: RESTful endpoints for integration

---

## 💻 CLI Commands

### Analyze Mode
```bash
# Basic analysis
db-optimizer analyze -d $DATABASE_URL

# JSON output
db-optimizer analyze -d $DATABASE_URL -f json -o report.json

# Filter by severity
db-optimizer analyze -d $DATABASE_URL -s high

# Limit queries analyzed
db-optimizer analyze -d $DATABASE_URL -n 50
```

### Agent Mode (Continuous Monitoring)
```bash
# Run as background agent
db-optimizer agent -d $DATABASE_URL \
  --interval-minutes 15 \
  --auto-apply \
  --dry-run false \
  --slack-webhook $SLACK_WEBHOOK
```

### Configuration Tuning
```bash
# Auto-detect workload and generate config
db-optimizer config-tune -d $DATABASE_URL

# Manual workload specification
db-optimizer config-tune --workload oltp --ram-gb 128 --cpus 32

# Show current config before recommendations
db-optimizer config-tune -d $DATABASE_URL --show-current

# Save to file
db-optimizer config-tune -d $DATABASE_URL -o postgresql.conf

# JSON output for automation
db-optimizer config-tune --workload web --ram-gb 64 -f json

# Validate current configuration
db-optimizer config-validate -d $DATABASE_URL
```

### Query Rewriting
```bash
# Rewrite from stdin
echo "SELECT * FROM users WHERE active = true" | db-optimizer rewrite-sql

# Rewrite from file
db-optimizer rewrite-sql -i query.sql

# Inline query
db-optimizer rewrite-sql -q "SELECT * FROM users"

# JSON output
db-optimizer rewrite-sql -i query.sql -f json
```

### HTTP Server
```bash
# Start API server
db-optimizer serve --port 8080

# With verbose logging
db-optimizer serve --port 8080 -v
```

---

## 🌐 HTTP API

**Base URL:** `http://localhost:8080/api/v1`

### Health & Stats
- `GET /health` - Health check
- `GET /stats/summary` - Statistics summary
- `GET /stats/metrics` - Prometheus metrics (includes eBPF event counters/drop/error gauges)

### Query Endpoints
- `GET /queries/slow` - List slow queries
- `GET /queries/:query_id` - Query details
- `POST /queries/analyze` - Analyze a query
- `POST /queries/rewrite` - Rewrite a query

### Recommendations
- `GET /recommendations` - List recommendations
- `GET /recommendations/:id` - Get recommendation
- `POST /recommendations/:id/approve` - Approve recommendation
- `POST /recommendations/:id/reject` - Reject recommendation

### Configuration Tuning
- `POST /config/tune` - Generate optimal configuration
- `GET /config/validate` - Validate current configuration
- `GET /config/detect-workload` - Detect workload type

### Lock Monitoring (eBPF)
- `GET /locks/contention` - Lock contention summary
- `GET /locks/summary` - Lock statistics
- `GET /locks/events` - Recent lock events
- `GET /locks/contention/events` - Raw contention events (waiting/blocking PIDs, query hashes)
- `GET /locks/lwlocks/events` - Recent LWLock events
- `GET /locks/rows/events` - Recent row lock events
- `GET /locks/table/:oid` - Table-specific lock info

**Prometheus eBPF metrics exposed via `/stats/metrics`:**
- `db_optimizer_ebpf_events_processed_total`
- `db_optimizer_ebpf_events_dropped_total`
- `db_optimizer_ebpf_perf_buffer_read_errors_total`

**Example Request:**
```bash
curl -X POST http://localhost:8080/api/v1/config/tune \
  -H "Content-Type: application/json" \
  -d '{"workload":"web","ram_gb":64,"cpu_count":16}'
```

---

## ⚙️ Configuration

Create `config.yml`:

```yaml
database:
  connection_string: "postgresql://user:pass@localhost/mydb"
  
http_server:
  enabled: true
  bind_address: "0.0.0.0"
  port: 8080
  enable_cors: true

auto_apply:
  enabled: false
  dry_run: true
  min_savings_threshold: 50.0
  max_risk_level: "low"
  verification_duration_secs: 300
  min_improvement_threshold: 10.0
  auto_rollback_on_failure: true
  slack_webhook: "${SLACK_WEBHOOK}"

ebpf:
  enabled: true
  sample_interval_ms: 1000
  enable_lock_tracking: true
  pid_map_refresh_ms: 1000
  pid_query_database_url: ""
  # Exposed endpoints:
  # - GET /locks/contention
  # - GET /locks/contention/events
  # - GET /stats/metrics (Prometheus, includes ebpf event counters)
```

See [config.example.yml](../config.example.yml) for full configuration options.

---

## 🔬 eBPF Integration

**Requirements:**
- Linux kernel 4.18+ with BTF enabled
- CAP_BPF capability (or root)
- libbpf installed

**Features:**
- Lock contention tracking at kernel level
- Process-level CPU/IO metrics
- Query → PID → metrics correlation
- Zero overhead when disabled

**Enable eBPF:**
```yaml
ebpf:
  enabled: true
  sample_interval_ms: 1000
  enable_lock_tracking: true
```

**View lock contention:**
```bash
curl http://localhost:8080/api/v1/locks/contention
curl http://localhost:8080/api/v1/locks/contention/events  # raw contention events (blocking/waiting PIDs)
```

---

## 🚢 Deployment

### Docker
```bash
docker build -t db-optimizer .
docker run -e DATABASE_URL=$DATABASE_URL -p 8080:8080 db-optimizer
```

### Docker Compose
```bash
docker-compose up
```

### Kubernetes
```bash
kubectl apply -f k8s/deployment.yaml
```

### Systemd
```bash
cd deployment/systemd
sudo ./install.sh
sudo systemctl start db-optimizer
```

See [DEPLOYMENT.md](../DEPLOYMENT.md) for detailed instructions.

---

## 🛠️ Development

### Build
```bash
cargo build
cargo test
```

### Run Tests
```bash
# All tests
cargo test

# Specific module
cargo test config_tuner

# With output
cargo test -- --nocapture
```

### eBPF Development
```bash
# Build eBPF programs
cd bpf
clang -O2 -target bpf -c locks.bpf.c -o locks.bpf.o
```

### Project Structure
```
db-optimizer/
├── src/
│   ├── main.rs                 # CLI entry point
│   ├── lib.rs                  # Library exports
│   ├── collector.rs            # PostgreSQL data collection
│   ├── analyzer.rs             # Query analysis
│   ├── recommendations.rs      # Recommendation engine
│   ├── config_tuner/           # Configuration tuning
│   │   ├── calculator.rs       # Config calculations
│   │   ├── validator.rs        # Config validation
│   │   └── workload_detector.rs# Workload detection
│   ├── query_rewriter/         # SQL optimization
│   │   ├── mod.rs
│   │   └── rules/              # Rewrite rules
│   ├── auto_apply/             # Automated remediation
│   │   ├── baseline.rs
│   │   ├── executor.rs
│   │   ├── verifier.rs
│   │   └── safety_filter.rs
│   ├── ebpf/                   # eBPF integration
│   │   ├── loader.rs
│   │   ├── lock_tracer.rs
│   │   └── correlator.rs
│   ├── http/                   # HTTP API
│   │   ├── mod.rs
│   │   ├── handlers/
│   │   └── models.rs
│   └── persistence/            # Data storage
│       ├── sqlite.rs
│       └── schema.rs
├── bpf/                        # eBPF programs
│   ├── locks.bpf.c
│   └── postgres.bpf.c
├── tests/                      # Integration tests
├── docs/                       # Documentation
│   ├── README.md              # This file
│   └── ROADMAP.md             # Future plans
└── deployment/                 # Deployment configs
    ├── docker/
    ├── k8s/
    └── systemd/
```

---

## 📊 Testing

**Test Coverage:**
- **33 config tuner tests** (calculator, validator, workload detector)
- **102 total tests** across all modules
- Integration tests for end-to-end workflows

**Run specific test suites:**
```bash
cargo test config_tuner::     # Config tuning tests
cargo test query_rewriter::   # Rewriter tests
cargo test auto_apply::       # Auto-apply tests
```

---

## 🤝 Contributing

1. Fork the repository
2. Create a feature branch
3. Make your changes
4. Add tests
5. Submit a pull request

---

## 📝 License

See LICENSE file for details.

---

## 🔗 Links

- [Main README](../README.md) - Installation and quick start
- [Examples](../examples/README.md) - **22 comprehensive examples covering all features**
- [Deployment Guide](../DEPLOYMENT.md) - Production deployment
- [Roadmap](ROADMAP.md) - Future plans and enhancements
- [Configuration Example](../config.example.yml) - Full config reference

## 📚 Examples Quick Links

**New to db-optimizer?** Check out our [comprehensive examples](../examples/README.md):
- [Quick Start](../examples/README.md#quick-start) - Get started in 5 minutes
- [Configuration Tuning](../examples/README.md#configuration-tuning) - 4 detailed examples
- [Query Analysis](../examples/README.md#query-analysis) - 2 examples with cost estimation
- [Query Rewriting](../examples/README.md#query-rewriting) - 3 optimization examples
- [Lock Monitoring](../examples/README.md#lock-monitoring) - eBPF lock tracking
- [Auto-Apply Mode](../examples/README.md#auto-apply-mode) - Safe automation examples
- [HTTP API](../examples/README.md#http-api-usage) - 3 API integration examples
- [Agent Mode](../examples/README.md#agent-mode) - Continuous monitoring setup
- [CI/CD Integration](../examples/README.md#cicd-integration) - GitHub Actions, Docker, K8s

---

**Version:** 0.1.0  
**Last Updated:** 2025-11-15
