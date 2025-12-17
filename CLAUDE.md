# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

`db-optimizer` (internally named `pg-query-advisor`) is a production-ready PostgreSQL performance optimizer that combines traditional database statistics with eBPF-powered system observability. It provides autonomous, workload-aware optimization recommendations and can automatically apply them with safety guardrails.

**Core Value Proposition:** Combines `pg_stat_statements`, table/index stats, and optional eBPF metrics to provide intelligent, cost-based performance recommendations with safe auto-apply capabilities.

## Build & Test Commands

### Build
```bash
# Debug build
cargo build

# Release build (optimized)
cargo build --release

# The binary is named 'db-optimizer' at runtime but package name is 'pg-query-advisor'
./target/release/db-optimizer --help
```

### Testing
```bash
# Run all tests
cargo test

# Run specific module tests
cargo test config_tuner::
cargo test query_rewriter::
cargo test auto_apply::

# Run with output visible
cargo test -- --nocapture

# Run single test
cargo test test_name

# With database (requires PostgreSQL running)
DATABASE_URL=postgresql://postgres:postgres@localhost:5432/testdb cargo test
```

### Code Quality
```bash
# Format code
cargo fmt

# Lint
cargo clippy -- -D warnings

# Security audit
cargo audit
```

### Local Development
```bash
# Run analyze mode
RUST_LOG=debug cargo run -- analyze -d $DATABASE_URL

# Run agent mode (continuous monitoring)
cargo run -- agent -d $DATABASE_URL --interval-minutes 15

# Run query rewriter
echo "SELECT * FROM users" | cargo run -- rewrite-sql

# Run config tuner
cargo run -- config-tune -d $DATABASE_URL

# Start HTTP server
cargo run -- serve --port 9090 -v
```

### Docker Development
```bash
# Start test PostgreSQL database
docker-compose up -d postgres

# Run one-shot analysis
docker-compose --profile analyze up db-optimizer-analyze

# Run as agent
docker-compose --profile agent up -d db-optimizer-agent

# View logs
docker-compose logs -f db-optimizer-agent
```

## Architecture

### High-Level Data Flow

```
Input → Collector → Analyzer → Recommendation Engine → Auto-Apply Pipeline → Output/Notifications
         ↓              ↓              ↓                     ↓
    pg_stat_stats  Cost Analysis  Index/Config/Query    Verify & Rollback
    Table/Index    Query Plans    Recommendations       Safety Checks
    eBPF Metrics   Heuristics     Risk Assessment       Slack/Webhook
```

### Key Architectural Concepts

1. **Trait-Based Architecture** (`src/traits.rs`): Core abstractions that enable pluggable implementations
   - `Collector`: Gathers metrics from database
   - `Analyzer`: Analyzes collected data
   - `RecommendationEngine`: Generates actionable recommendations
   - Adapters in `src/adapters/` implement these traits

2. **Operation Modes** (defined in `src/config.rs`):
   - `observe_only`: Read-only analysis, no database changes
   - `safe_auto`: Auto-applies only safe operations (CREATE INDEX CONCURRENTLY)
   - `aggressive`: More aggressive optimizations (use with caution)

3. **Auto-Apply Pipeline** (`src/auto_apply/`):
   - **Baseline** (`baseline.rs`): Captures performance metrics before changes
   - **Safety Filter** (`safety_filter.rs`, `enhanced_safety.rs`): Governance rules and risk assessment
   - **Executor** (`executor.rs`): Applies changes using appropriate SQL
   - **Verifier** (`verifier.rs`, `enhanced_verifier.rs`): Compares metrics, triggers rollback if needed
   - **Notifier** (`notifier.rs`): Sends Slack/webhook notifications

4. **Config Tuner** (`src/config_tuner/`):
   - **Workload Detector** (`workload_detector.rs`): Auto-detects workload type (Desktop, Web, OLTP, Data Warehouse, Mixed)
   - **Calculator** (`calculator.rs`): PGTune-inspired calculations for 15+ PostgreSQL parameters
   - **Validator** (`validator.rs`): Safety validators prevent OOM and invalid configurations

5. **Query Rewriter** (`src/query_rewriter/`):
   - 8+ optimization rules in `rules/` directory
   - Rules include: SELECT * removal, predicate pushdown, subquery flattening, redundant join removal, LIMIT addition
   - Both basic and enhanced versions of key rules

6. **eBPF Integration** (`src/ebpf/`):
   - Optional kernel-level observability (requires Linux kernel 5.4+, CAP_BPF)
   - Lock contention tracking, CPU/IO metrics
   - Zero overhead when disabled

7. **HTTP API** (`src/http/`):
   - RESTful endpoints for integration
   - Health checks, query analysis, config tuning, lock monitoring
   - Prometheus metrics export

8. **Persistence Layer** (`src/persistence/`):
   - SQLite or PostgreSQL backend for historical data
   - Tracks applied recommendations, trends, audit trail

### Module Responsibilities

- **`src/main.rs`**: CLI entry point, command definitions (analyze, agent, rewrite-sql, serve, config-tune, config-validate)
- **`src/lib.rs`**: Library exports for all public APIs
- **`src/collector.rs`**: Generic collection interface
- **`src/analyzer.rs`**: Query analysis logic
- **`src/recommendations.rs`**: Recommendation generation
- **`src/models.rs`**: Core data structures (Query, Recommendation, AnalysisReport, etc.)
- **`src/config.rs`**: Configuration loading and validation (uses config.yml)
- **`src/http_server.rs`**: HTTP server initialization

### Important Implementation Details

**Binary vs Package Name:** The package is named `pg-query-advisor` (in Cargo.toml) but the binary is `db-optimizer`. This is intentional for branding.

**Database Connection:** Uses `sqlx` with PostgreSQL. Requires `pg_stat_statements` extension enabled for query analysis.

**Safety by Default:**
- Protected schemas/tables defined in config
- Maintenance windows for DDL operations
- CREATE INDEX CONCURRENTLY for zero-downtime indexing
- Baseline capture and verification before accepting changes
- Automatic rollback on performance regression

**Risk Levels:** All recommendations have risk assessments (low, medium, high, critical) that control auto-apply behavior.

**eBPF Programs:** Located in `bpf/` directory (C programs compiled to BPF bytecode).

## Configuration

The tool uses `config.yml` (see `config.example.yml` for full reference). Key sections:

- `database`: Connection settings
- `operation_mode`: Controls behavior (observe_only, safe_auto, aggressive)
- `safety`: Protected resources, size limits, constraint protection
- `maintenance_window`: When DDL operations can execute
- `auto_apply`: Auto-apply settings, thresholds, verification
- `ebpf`: eBPF configuration
- `persistence`: Storage backend (SQLite/Postgres)
- `http_server`: API server settings
- `notifications`: Slack/webhook URLs

## Testing Strategy

- **Unit tests**: In each module (33 config tuner tests, 102 total)
- **Integration tests**: In `tests/` directory
- **Test fixtures**: `scripts/test-data.sql`, `scripts/test-queries.sql`
- **CI/CD**: GitHub Actions workflow at `.github/workflows/ci.yml`
  - Runs tests, clippy, formatting checks
  - Builds Docker image
  - Security audit with cargo-audit
  - Code coverage with tarpaulin

## Deployment Options

1. **Cargo/Binary**: Direct execution of compiled binary
2. **Docker**: Single container with all dependencies
3. **Docker Compose**: Multi-container setup with PostgreSQL
4. **Kubernetes**: Manifests in `k8s/` (standalone or sidecar)
5. **Systemd**: Service files in `deployment/systemd/`

See `DEPLOYMENT.md` for detailed production deployment guidance.

## Development Workflow

1. **Adding a new recommendation type:**
   - Add variant to `IssueType` in `src/models.rs`
   - Implement detection logic in `src/analyzer.rs`
   - Add recommendation generation in `src/recommendations.rs`
   - Update auto-apply executor if needed (`src/auto_apply/executor.rs`)

2. **Adding a query rewrite rule:**
   - Create new rule file in `src/query_rewriter/rules/`
   - Implement rule logic
   - Register in `src/query_rewriter/mod.rs`
   - Add tests

3. **Adding HTTP endpoint:**
   - Add handler in `src/http/handlers/`
   - Register route in `src/http/mod.rs`
   - Update API documentation

4. **Modifying configuration:**
   - Update structs in `src/config.rs`
   - Update `config.example.yml`
   - Update validation logic
   - Update documentation

## Common Gotchas

- The binary name (`db-optimizer`) differs from package name (`pg-query-advisor`)
- eBPF features only work on Linux with appropriate kernel version and capabilities
- `pg_stat_statements` extension must be enabled in PostgreSQL for query analysis
- Auto-apply in production should start with `dry_run: true` and `operation_mode: observe_only`
- Config tuner calculations assume you have full control over PostgreSQL configuration
- Some tests require a running PostgreSQL instance (use docker-compose for local testing)

## Key Environment Variables

- `DATABASE_URL`: PostgreSQL connection string
- `SLACK_WEBHOOK` or `DBO_SLACK_WEBHOOK`: Slack notification webhook
- `RUST_LOG`: Logging level (e.g., `debug`, `info`, `warn`)
- `POSTGRES_PASSWORD`: Used by Docker Compose setup
