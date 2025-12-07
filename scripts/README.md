# PostgreSQL Query Health Advisor - Test Scripts

This directory contains scripts to test the PostgreSQL Query Health Advisor with realistic workloads.

## 📁 Files

### `run-test.sh` ⭐
**Automated test runner** - Run this to see the full demo!

Automatically:
- Sets up PostgreSQL with pg_stat_statements
- Loads test data (4.5M rows)
- Runs problematic queries
- Executes the advisor
- Shows recommendations

**Usage:**
```bash
./scripts/run-test.sh
```

**Duration:** ~3-5 minutes (mostly data loading)

### `test-data.sql`
**Test database schema and data generator**

Creates a realistic e-commerce database:
- **500,000 customers** with email, name, country, city
- **10,000 products** with categories and prices
- **1,000,000 orders** with status and amounts
- **3,000,000 order items** (line items for orders)

**Intentional Performance Issues:**
- ❌ No index on `customers.email` (frequently queried)
- ❌ No index on `orders.customer_id` (foreign key, no index!)
- ❌ No index on `orders.status` (frequently filtered)
- ❌ No index on `order_items.order_id` (foreign key, no index!)
- ❌ No index on `order_items.product_id` (foreign key, no index!)
- ⚠️ Unused index on `products.created_at` (never queried)

**Manual Usage:**
```bash
docker exec -i test-postgres psql -U postgres < scripts/test-data.sql
```

### `test-queries.sql`
**Problematic query workload**

Contains 8 different slow queries that will trigger recommendations:
- Customer lookup by email (no index)
- Orders by customer (missing FK index)
- Order status filtering (no index)
- Complex joins without proper indexes
- Product category searches (no index)

**Manual Usage:**
```bash
# Run once
docker exec -i test-postgres psql -U postgres < scripts/test-queries.sql

# Run multiple times to accumulate statistics
for i in {1..20}; do
  docker exec -i test-postgres psql -U postgres < scripts/test-queries.sql > /dev/null 2>&1
done
```

## 🚀 Quick Start

### Option 1: Automated (Recommended)
```bash
./scripts/run-test.sh
```

### Option 2: Manual Step-by-Step
```bash
# 1. Start PostgreSQL with pg_stat_statements
docker run --name test-postgres \
  -e POSTGRES_PASSWORD=postgres \
  -p 5432:5432 \
  -d postgres:15 \
  -c shared_preload_libraries=pg_stat_statements

# 2. Wait for startup
sleep 5

# 3. Enable extension
docker exec test-postgres psql -U postgres -c "CREATE EXTENSION pg_stat_statements;"

# 4. Load test data (takes 2-3 minutes)
docker exec -i test-postgres psql -U postgres < scripts/test-data.sql

# 5. Run queries multiple times
for i in {1..20}; do
  docker exec -i test-postgres psql -U postgres < scripts/test-queries.sql > /dev/null 2>&1
done

# 6. Run the advisor
./target/release/pg-query-advisor --database-url postgresql://postgres:postgres@localhost:5432/postgres
```

## 📊 Expected Results

The advisor should detect:

### Missing Indexes (HIGH Priority)
- `CREATE INDEX CONCURRENTLY idx_customers_email ON customers(email);`
- `CREATE INDEX CONCURRENTLY idx_orders_customer_id ON orders(customer_id);`
- `CREATE INDEX CONCURRENTLY idx_orders_status ON orders(status);`
- `CREATE INDEX CONCURRENTLY idx_order_items_order_id ON order_items(order_id);`
- `CREATE INDEX CONCURRENTLY idx_order_items_product_id ON order_items(product_id);`

### Unused Indexes (MEDIUM Priority)
- `DROP INDEX IF EXISTS idx_products_created_at;` (never queried)

### Slow Queries (CRITICAL)
- Queries with sequential scans on large tables
- Joins without proper indexes
- High-volume queries needing optimization

### Cost Savings
- Estimated monthly savings: **$200-500+** (depending on database size and query volume)

## 🧹 Cleanup

```bash
# Stop and remove test container
docker stop test-postgres && docker rm test-postgres
```

## 📝 Notes

- **Data Size:** ~4.5 million rows, ~500MB database
- **Load Time:** 2-3 minutes on modern hardware
- **Query Time:** Each query iteration takes ~5-10 seconds
- **Requirements:** Docker, PostgreSQL 12+, 2GB+ available RAM

## 🎯 Testing Tips

1. **Start Simple:** Run `./scripts/run-test.sh` first to see everything work
2. **Experiment:** Modify `test-queries.sql` to test different query patterns
3. **Scale Up:** Increase row counts in `test-data.sql` for larger tests
4. **Real Data:** Replace with your own schema to test production patterns

## 🐛 Troubleshooting

### "pg_stat_statements must be loaded via shared_preload_libraries"
**Solution:** Start PostgreSQL with the correct flag:
```bash
docker run ... -c shared_preload_libraries=pg_stat_statements
```

### "Connection refused"
**Solution:** Wait longer for PostgreSQL to start:
```bash
sleep 10  # instead of sleep 5
```

### "No recommendations found"
**Solution:** Run more query iterations or use slower queries:
```bash
# Run 50+ times instead of 20
for i in {1..50}; do ...
```

## 📚 Further Reading

- Main README: `../README.md`
- Source code: `../src/`
- Tests: `../tests/`
