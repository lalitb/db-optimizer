# Copilot Instructions for db-optimizer

PostgreSQL performance optimizer combining database statistics with optional eBPF observability. Provides autonomous, workload-aware optimization recommendations with safe auto-apply capabilities.

## Build & Test

```bash
# Build
cargo build                    # Debug
cargo build --release          # Release (binary: target/release/db-optimizer)

# Test
cargo test                     # All tests
cargo test test_name           # Single test
cargo test config_tuner::      # Module tests
cargo test -- --nocapture      # With output visible

# With database (requires PostgreSQL + pg_stat_statements extension)
DATABASE_URL=postgresql://postgres:postgres@localhost:5432/testdb cargo test

# Code quality
cargo fmt                      # Format
cargo clippy -- -D warnings    # Lint
cargo audit                    # Security audit

# Run locally
docker-compose up -d postgres  # Start test database
cargo run -- analyze -d $DATABASE_URL
cargo run -- serve --port 9090 -v
```

## Architecture

### Data Flow
```
Collector → Analyzer → Recommendation Engine → Auto-Apply Pipeline → Output
    ↓           ↓              ↓                      ↓
pg_stat_*   Cost Analysis  Index/Config/Query    Verify & Rollback
eBPF Metrics  Heuristics   Risk Assessment       Notifications
```

### Trait-Based Design (`src/traits.rs`)
Core abstractions enable pluggable implementations:
- `MetricsCollector` - Gathers database statistics
- `Analyzer` - Analyzes collected data
- `RecommendationGenerator` - Generates actionable recommendations
- `AutoApplyOrchestrator` - Executes changes with safety checks
- `SystemMetricsCollector` - Optional eBPF/system metrics

Implementations live in `src/adapters/`.

### Key Subsystems

**Auto-Apply Pipeline** (`src/auto_apply/`):
- `baseline.rs` - Captures metrics before changes
- `safety_filter.rs`, `enhanced_safety.rs` - Governance and risk assessment
- `executor.rs` - Applies SQL changes
- `verifier.rs`, `enhanced_verifier.rs` - Validates improvements, triggers rollback
- `notifier.rs` - Slack/webhook notifications

**Config Tuner** (`src/config_tuner/`):
- `workload_detector.rs` - Auto-detects workload type (OLTP, Data Warehouse, etc.)
- `calculator.rs` - PGTune-inspired calculations for PostgreSQL parameters
- `validator.rs` - Prevents OOM and invalid configurations

**Query Rewriter** (`src/query_rewriter/rules/`):
- SELECT * removal, predicate pushdown, subquery flattening, redundant join removal, LIMIT addition

**eBPF Integration** (`src/ebpf/`):
- Optional kernel-level observability (Linux 5.4+, CAP_BPF required)
- Zero overhead when disabled

### Operation Modes (in `config.yml`)
- `observe_only` - Read-only analysis
- `safe_auto` - Auto-applies only safe operations (CREATE INDEX CONCURRENTLY)
- `aggressive` - More aggressive optimizations

## Key Conventions

### Binary vs Package Name
Package is `pg-query-advisor` (Cargo.toml) but binary is `db-optimizer`. This is intentional.

### Adding Features

**New recommendation type:**
1. Add variant to `IssueType` in `src/models.rs`
2. Implement detection in `src/analyzer.rs`
3. Add generation in `src/recommendations.rs`
4. Update `src/auto_apply/executor.rs` if needed

**New query rewrite rule:**
1. Create rule file in `src/query_rewriter/rules/`
2. Register in `src/query_rewriter/mod.rs`

**New HTTP endpoint:**
1. Add handler in `src/http/handlers/`
2. Register route in `src/http/mod.rs`

**Configuration changes:**
1. Update structs in `src/config.rs`
2. Update `config.example.yml`

### Safety Patterns
- All recommendations have risk levels (low, medium, high, critical)
- Protected schemas/tables defined in config
- CREATE INDEX CONCURRENTLY for zero-downtime indexing
- Baseline capture and verification before accepting changes
- Automatic rollback on performance regression

### Database Requirements
- PostgreSQL with `pg_stat_statements` extension enabled
- Run `ANALYZE` regularly for accurate statistics

## Environment Variables

- `DATABASE_URL` - PostgreSQL connection string
- `RUST_LOG` - Logging level (debug, info, warn)
- `SLACK_WEBHOOK` or `DBO_SLACK_WEBHOOK` - Notification webhook

## Feature Flags

```bash
cargo build --features ebpf   # Enable eBPF support (Linux only)
cargo build --features cnpg   # Enable CloudNativePG plugin
```
