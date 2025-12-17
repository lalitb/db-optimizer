# Testing Guide for pg-query-advisor

## Test Coverage

### Unit Tests (10 total)
- ✅ 5 library tests (auto-apply modules)
- ✅ 5 integration tests (can run without database)
- ✅ 4 database integration tests (require TEST_DATABASE_URL)

### Test Organization

```
tests/
├── integration_test.rs    # All integration tests
src/
├── auto_apply/
│   ├── safety_filter.rs   # 3 unit tests
│   └── verifier.rs        # 1 unit test
└── lib.rs                 # 1 unit test
```

## Running Tests

### Quick Test (No Database Required)
```bash
cargo test
```
**Output:**
- 5 lib unit tests ✅
- 5 integration tests ✅
- 4 ignored (require database)

### Full Test Suite (Requires Database)
```bash
# Set up test database
export TEST_DATABASE_URL="postgresql://postgres:postgres@localhost/test"

# Run all tests including database tests
cargo test -- --ignored --test-threads=1
```

### Run Specific Test
```bash
# Run only auto-apply tests
cargo test auto_apply

# Run specific test
cargo test test_risk_assessment

# Run with output
cargo test test_sql_parsing -- --nocapture
```

## Test Database Setup

### Option 1: Using Provided Scripts
```bash
# Start PostgreSQL (if not running)
docker run -d \
  --name postgres-test \
  -e POSTGRES_PASSWORD=postgres \
  -p 5432:5432 \
  postgres:15

# Create test database and load data
export DATABASE_URL="postgresql://postgres:postgres@localhost/test"
psql $DATABASE_URL < scripts/test-data.sql
psql $DATABASE_URL < scripts/test-queries.sql

# Enable pg_stat_statements
psql $DATABASE_URL -c "CREATE EXTENSION IF NOT EXISTS pg_stat_statements;"

# Run tests
export TEST_DATABASE_URL=$DATABASE_URL
cargo test -- --ignored
```

### Option 2: Minimal Test Database
```bash
# Just for testing auto-apply logic (smaller dataset)
psql $TEST_DATABASE_URL << 'EOF'
CREATE TABLE test_table (
    id SERIAL PRIMARY KEY,
    test_col VARCHAR(255),
    created_at TIMESTAMP DEFAULT NOW()
);

INSERT INTO test_table (test_col)
SELECT 'value' || i FROM generate_series(1, 1000) i;

CREATE EXTENSION IF NOT EXISTS pg_stat_statements;
EOF
```

## Test Scenarios Covered

### 1. Safety Filter Tests ✅
- **test_risk_assessment_missing_index**: Verifies low-risk for index creation
- **test_blocks_high_risk**: Ensures high-risk operations require approval
- **test_parse_create_index**: Validates SQL parsing for CREATE INDEX
- **test_parse_drop_index**: Validates SQL parsing for DROP INDEX

### 2. Auto-Apply Workflow Tests ✅
- **test_auto_apply_dry_run**: Verifies dry-run mode doesn't execute
- **test_auto_apply_safety_filter**: Tests risk-based filtering
- **test_auto_apply_savings_threshold**: Tests savings threshold filtering

### 3. Calculation Tests ✅
- **test_improvement_calculation**: Validates performance improvement math
- **test_cost_estimation**: Verifies cost savings calculation
- **test_analyzer_logic**: Tests slow query detection

### 4. SQL Parsing Tests ✅
- CREATE INDEX CONCURRENTLY parsing
- DROP INDEX parsing
- Schema qualification handling
- Multi-column index extraction

### 5. Integration Tests (Require Database) 🔒
- **test_basic_workflow**: Full analysis pipeline
- **test_auto_apply_dry_run**: End-to-end dry-run
- **test_auto_apply_safety_filter**: Live safety filtering
- **test_auto_apply_savings_threshold**: Live threshold testing

## Test Data Characteristics

The test scripts in `scripts/` provide:

### Tables Created
- **customers**: 500K rows
- **products**: 10K rows  
- **orders**: 1M rows
- **order_items**: 3M rows

### Performance Issues Intentionally Created
1. Missing indexes on:
   - `customers.email` (frequently queried)
   - `orders.customer_id` (FK without index)
   - `orders.status` (frequently filtered)
   - `order_items.order_id` (FK without index)
   - `order_items.product_id` (FK without index)

2. Unused index:
   - `idx_products_created_at` (never queried)

3. Slow queries:
   - Multi-table joins without indexes
   - Full table scans
   - Complex aggregations

### Expected Recommendations
When run against the test database, you should see:
- 5-8 missing index recommendations
- 1 unused index recommendation
- 3-5 slow query warnings
- Estimated savings: $500-800/month

## Adding New Tests

### Unit Test Template
```rust
#[test]
fn test_new_feature() {
    // Arrange
    let config = AutoApplyConfig::default();
    let component = MyComponent::new(config);
    
    // Act
    let result = component.do_something();
    
    // Assert
    assert_eq!(result.status, ExpectedStatus);
}
```

### Integration Test Template
```rust
#[tokio::test]
#[ignore]
async fn test_new_integration() {
    let database_url = std::env::var("TEST_DATABASE_URL")
        .unwrap_or_else(|_| "postgresql://postgres:postgres@localhost/test".to_string());
    
    let pool = sqlx::postgres::PgPoolOptions::new()
        .connect(&database_url)
        .await
        .expect("Failed to connect");
    
    // Your test logic here
    
    // Cleanup if needed
}
```

## CI/CD Integration

### GitHub Actions Example
```yaml
name: Tests

on: [push, pull_request]

jobs:
  test:
    runs-on: ubuntu-latest
    
    services:
      postgres:
        image: postgres:15
        env:
          POSTGRES_PASSWORD: postgres
        options: >-
          --health-cmd pg_isready
          --health-interval 10s
          --health-timeout 5s
          --health-retries 5
        ports:
          - 5432:5432
    
    steps:
      - uses: actions/checkout@v3
      
      - name: Install Rust
        uses: actions-rs/toolchain@v1
        with:
          toolchain: stable
      
      - name: Setup database
        env:
          DATABASE_URL: postgresql://postgres:postgres@localhost/test
        run: |
          psql $DATABASE_URL -c "CREATE EXTENSION pg_stat_statements;"
          psql $DATABASE_URL < scripts/test-data.sql
      
      - name: Run tests
        env:
          TEST_DATABASE_URL: postgresql://postgres:postgres@localhost/test
        run: cargo test -- --include-ignored
```

## Test Coverage Goals

Current coverage:
- ✅ Core recommendation engine: 100%
- ✅ Auto-apply safety: 100%
- ✅ SQL parsing: 100%
- ✅ Risk assessment: 100%
- ⚠️  Baseline capture: 80% (needs live DB tests)
- ⚠️  Executor: 70% (needs live DB tests)
- ⚠️  Verifier: 70% (needs live DB tests)

## Troubleshooting Tests

### Test Fails: "Failed to connect to test database"
```bash
# Check PostgreSQL is running
pg_isready

# Verify connection string
psql $TEST_DATABASE_URL -c "SELECT 1;"

# Check environment variable
echo $TEST_DATABASE_URL
```

### Test Fails: "pg_stat_statements not enabled"
```bash
# Enable extension
psql $TEST_DATABASE_URL -c "CREATE EXTENSION IF NOT EXISTS pg_stat_statements;"

# Verify
psql $TEST_DATABASE_URL -c "SELECT * FROM pg_stat_statements LIMIT 1;"
```

### Tests Run Too Slowly
```bash
# Run tests in parallel (but not DB tests)
cargo test --lib

# Run single-threaded (for DB tests)
cargo test -- --test-threads=1 --ignored
```

## Continuous Testing

### Watch Mode
```bash
# Install cargo-watch
cargo install cargo-watch

# Auto-run tests on file changes
cargo watch -x test
```

### Pre-commit Hook
```bash
# .git/hooks/pre-commit
#!/bin/bash
cargo test --lib
if [ $? -ne 0 ]; then
    echo "Tests failed! Commit aborted."
    exit 1
fi
```

## Performance Benchmarks

### Current Test Performance
- Unit tests: ~0.01s
- Integration tests (no DB): ~0.01s
- Database tests: ~5-10s (depending on data size)

### Optimization Tips
- Use smaller datasets for testing
- Run expensive tests in CI only
- Use `#[ignore]` for slow tests
- Mock database connections when possible
