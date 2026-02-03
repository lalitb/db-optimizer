# Vaanyze Benchmark Pack

A repeatable, automated benchmark suite for validating Vaanyze's index recommendation and safe apply capabilities.

> ⚠️ **BENCHMARK-ONLY - NOT PART OF PRODUCT SAFETY MODEL**
>
> This benchmark harness runs DDL (CREATE/DROP INDEX) directly against a test database
> to create controlled scenarios. This is intentional for testing purposes.
>
> **Important distinctions:**
> - Benchmark scripts bypass Vaanyze's OperationGuard and safety controls
> - They execute DDL directly via `psql`, not through Vaanyze API
> - Results demonstrate what Vaanyze *can detect*, not what it *will execute*
> - Production safety guarantees only apply when using Vaanyze HTTP API with proper mode
>
> **Never run these scripts against production databases.**

## Quick Start

```bash
# 1. Start the benchmark PostgreSQL
cd examples/benchmark-pack
docker-compose up -d postgres

# 2. Run all scenarios
make bench-all

# 3. View results
cat examples/benchmark-pack/results/summary.json
```

## Prerequisites

- Docker and Docker Compose
- `psql` client (for running scenarios outside container)
- Python 3 (for JSON formatting)
- `curl` (for Vaanyze API calls)

## Directory Structure

```
examples/benchmark-pack/
├── docker-compose.yml      # Postgres + load generator setup
├── init.sql                # pg_stat_statements initialization
├── scenarios/              # Individual test scenarios
│   ├── 01-missing-index-where/
│   │   ├── schema.sql      # Table creation + data
│   │   ├── workload.sql    # Queries to run
│   │   └── config.json     # Scenario configuration
│   ├── 02-composite-index/
│   ├── 03-join-path/
│   ├── 04-order-by-limit/
│   ├── 05-write-heavy-rollback/
│   └── 06-wal-pressure-negative/
└── results/                # Benchmark output (gitignored)
    ├── 01-missing-index-where.json
    ├── ...
    └── summary.json

scripts/bench/
├── run_all.sh              # Run all scenarios
└── run_scenario.sh         # Run single scenario
```

## Running Benchmarks

### Option 1: Full Suite (Makefile)

```bash
# From project root
make bench-up           # Start PostgreSQL
make bench-all          # Run all scenarios
make bench-down         # Stop and cleanup
```

### Option 2: Single Scenario

```bash
make scenario-missing-index
make scenario-order-by-limit
# etc.
```

### Option 3: Manual Execution

```bash
# Start Postgres
cd examples/benchmark-pack
docker-compose up -d postgres

# Wait for healthy
docker-compose ps

# Run scenario directly
../../../scripts/bench/run_scenario.sh 01-missing-index-where
```

### Option 4: With Vaanyze Service

```bash
# Start everything including Vaanyze
docker-compose --profile with-vaanyze up -d

# Run scenarios (will call real Vaanyze API)
../../scripts/bench/run_all.sh
```

## Scenarios

### 1. Missing Index on WHERE Predicate
- **Table**: `users` (100K rows)
- **Issue**: Sequential scan on `email` equality lookup
- **Expected**: Index on `users(email)` recommended
- **Validation**: Seq Scan → Index Scan

### 2. Composite Index (tenant_id, created_at)
- **Table**: `events` (500K rows)
- **Issue**: Multi-tenant time-range queries
- **Expected**: Index on `events(tenant_id, created_at)` recommended
- **Validation**: Index covers both filter columns

### 3. Join Path (Missing FK Index)
- **Tables**: `customers` (10K), `orders` (200K)
- **Issue**: Hash join instead of index nested loop
- **Expected**: Index on `orders(customer_id)` recommended
- **Validation**: Join uses index

### 4. ORDER BY + LIMIT
- **Table**: `products` (50K rows)
- **Issue**: Sort operation for pagination
- **Expected**: Index on `products(price)` recommended
- **Validation**: Avoids sort by using index

### 5. Write-Heavy Rollback Test
- **Table**: `metrics` (100K rows, high write rate)
- **Issue**: Index may slow writes more than it helps reads
- **Expected**: Rollback triggered due to write regression
- **Validation**: Automatic rollback occurs

### 6. WAL Pressure Negative Test
- **Table**: `audit_log` (200K rows)
- **Issue**: Append-only workload, no selective reads
- **Expected**: NO index recommendation (negative test)
- **Validation**: No inappropriate recommendations

## Results Format

Each scenario produces a JSON file:

```json
{
  "scenario": "01-missing-index-where",
  "started_at": "2024-01-15T10:30:00Z",
  "completed_at": "2024-01-15T10:32:45Z",
  "duration_secs": 165,
  
  "baseline": {
    "total_queries": 50,
    "slowest_query": {
      "queryid": 12345,
      "query": "SELECT id, email... WHERE email = ...",
      "calls": 50,
      "mean_exec_time_ms": 245.67,
      "total_exec_time_ms": 12283.50
    }
  },
  
  "recommendation": {
    "table": "users",
    "column": "email",
    "index_name": "idx_bench_users_email",
    "create_sql": "CREATE INDEX CONCURRENTLY idx_bench_users_email ON users(email)"
  },
  
  "explain_proof": {
    "before_plan_summary": "Seq Scan on users ...",
    "after_plan_summary": "Index Scan using idx_bench_users_email ...",
    "index_used": true
  },
  
  "apply_result": {
    "applied": true,
    "index_name": "idx_bench_users_email",
    "create_duration_ms": 1250
  },
  
  "verification": {
    "idx_scan_delta": 50,
    "post_apply_stats": {
      "slowest_query": {
        "mean_exec_time_ms": 0.85
      }
    }
  }
}
```

## Interpreting Results

### Success Criteria

| Metric | Good | Warning | Failed |
|--------|------|---------|--------|
| `index_used` | `true` | - | `false` |
| `idx_scan_delta` | `> 0` | `0` | - |
| Cost improvement | `> 50%` | `10-50%` | `< 10%` |

### Console Output

```
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
Scenario Summary: 01-missing-index-where
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

  Index created:     true
  Index used:        Yes
  Index scans:       50
  Duration:          45s
  Results:           examples/benchmark-pack/results/01-missing-index-where.json

✓ Scenario PASSED
```

## Configuration

### Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `PGHOST` | `localhost` | PostgreSQL host |
| `PGPORT` | `5433` | PostgreSQL port |
| `PGUSER` | `postgres` | PostgreSQL user |
| `PGPASSWORD` | `postgres` | PostgreSQL password |
| `PGDATABASE` | `benchdb` | Database name |
| `VAANYZE_URL` | `http://localhost:9090` | Vaanyze API URL |

### Scenario Config (config.json)

```json
{
  "scenario": "missing-index-where",
  "target_table": "users",
  "target_column": "email",
  "expected_recommendation": "CREATE INDEX ON users(email)",
  "pgbench_options": {
    "clients": 4,
    "jobs": 2,
    "time_seconds": 30
  },
  "validation": {
    "index_must_be_used": true,
    "min_cost_improvement_pct": 50
  }
}
```

## Troubleshooting

### PostgreSQL won't start
```bash
# Check logs
docker-compose logs postgres

# Reset volume
docker-compose down -v
docker-compose up -d postgres
```

### pg_stat_statements not available
```bash
# Verify extension
docker-compose exec postgres psql -U postgres -d benchdb \
  -c "SELECT * FROM pg_extension WHERE extname = 'pg_stat_statements';"

# Check shared_preload_libraries
docker-compose exec postgres psql -U postgres -d benchdb \
  -c "SHOW shared_preload_libraries;"
```

### Scenario fails to connect
```bash
# Test connection
psql -h localhost -p 5433 -U postgres -d benchdb -c "SELECT 1"

# Check firewall/network
docker-compose exec postgres pg_isready
```

## Extending

### Adding a New Scenario

1. Create directory: `scenarios/07-my-scenario/`
2. Add `schema.sql` - table creation and data generation
3. Add `workload.sql` - queries to run
4. Add `config.json` - scenario metadata
5. Update `scripts/bench/run_all.sh` scenarios list

### Custom Load Patterns

Modify `workload.sql` to include custom query patterns. Use deterministic data (no `random()` in schema) for reproducibility.

## Cleanup

```bash
# Stop containers
docker-compose down

# Remove volumes (full reset)
docker-compose down -v

# Remove results
rm -rf examples/benchmark-pack/results/*
```
