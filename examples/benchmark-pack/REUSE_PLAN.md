# Vaanyze Benchmark Pack - Repo Reuse Plan

## Existing Assets Found

### Docker Compose
- **`./docker-compose.yml`** - Postgres 16-alpine with test data init
  - ✅ REUSE: Base postgres setup pattern
  - ⚠️ MISSING: `shared_preload_libraries=pg_stat_statements`, `track_io_timing`
  - ⚠️ MISSING: Load generator container

### Benchmark Scripts
- **`./scripts/benchmark.sh`** - Performance benchmarks for build/tests
  - ⚠️ NOT USEFUL: Only tests Rust compilation, not database scenarios

### Test Infrastructure
- **`./scripts/run-test.sh`** - Full test script
  - ✅ REUSE: Postgres docker setup with pg_stat_statements
  - ✅ REUSE: Test data loading pattern
  - ✅ REUSE: Query workload pattern
  - ⚠️ NOT USEFUL: Uses external docker container, not compose

### Test Data
- **`./scripts/test-data.sql`** - E-commerce schema (customers, products, orders, order_items)
  - ✅ REUSE: Schema is good for missing index scenarios
  - ⚠️ MODIFY: Need deterministic seeds (currently uses random())
  - ⚠️ MODIFY: Need smaller scale for benchmark repeatability

- **`./scripts/test-queries.sql`** - Slow queries designed for testing
  - ✅ REUSE: Query patterns for missing index detection
  - ✅ REUSE: Multi-table join scenarios

### Validation Pipeline (src/analyzer/validation/)
- **`src/http/handlers/validation.rs`** - HTTP endpoints
  - ✅ ENDPOINTS EXIST:
    - `POST /api/v1/index/validate` - EXPLAIN proof
    - `POST /api/v1/index/apply` - Apply with verification
    - `POST /api/v1/index/rollback` - Rollback
    - `GET /api/v1/index/status/:id` - Status
    - `GET /api/v1/index/audit` - Audit log
  - ⚠️ NOT WIRED: Routes not added to main router yet

### HTTP Server
- **`src/http/mod.rs`** - Router configuration
  - ✅ REUSE: Existing endpoint patterns
  - ⚠️ MISSING: Validation endpoints not wired to router

### Integration Tests
- **`tests/integration_test.rs`** - Comprehensive tests
  - ✅ REUSE: Test patterns for collector, analyzer, auto-apply
  - ✅ REUSE: Database connection patterns

## Reuse Strategy

1. **Schema**: Adapt `scripts/test-data.sql` with deterministic seeds and smaller scale
2. **Queries**: Use patterns from `scripts/test-queries.sql`
3. **Docker**: Build on `docker-compose.yml` with proper pg_stat_statements config
4. **Endpoints**: Wire validation handlers to router, then use them in benchmark
5. **Test Patterns**: Follow patterns from `tests/integration_test.rs`

## What Needs to Be Built

1. **docker-compose.yml** for benchmark-pack with:
   - Postgres with proper config
   - pgbench or Python load generator
   - Optional Vaanyze container

2. **6 Scenario SQL files** with deterministic data

3. **Load generator** using pgbench custom scripts

4. **Scenario runner** that:
   - Sets up schema/data
   - Runs baseline load
   - Captures metrics
   - Calls Vaanyze HTTP endpoints (or stubs if not wired)
   - Validates results

5. **Results JSON format** per spec

6. **Makefile targets** for easy execution
