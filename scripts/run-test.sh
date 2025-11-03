#!/bin/bash
# PostgreSQL Query Health Advisor - Full Test Script

set -e

echo "🚀 PostgreSQL Query Health Advisor - Full Test"
echo "================================================"
echo ""

# Colors
GREEN='\033[0;32m'
BLUE='\033[0;34m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Stop and remove old container if exists
echo -e "${BLUE}Cleaning up old test container...${NC}"
docker stop test-postgres 2>/dev/null || true
docker rm test-postgres 2>/dev/null || true

# Start PostgreSQL with pg_stat_statements
echo -e "${BLUE}Starting PostgreSQL with pg_stat_statements...${NC}"
docker run --name test-postgres \
  -e POSTGRES_PASSWORD=postgres \
  -p 5432:5432 \
  -d postgres:15 \
  -c shared_preload_libraries=pg_stat_statements \
  -c pg_stat_statements.track=all

# Wait for PostgreSQL to be ready
echo -e "${BLUE}Waiting for PostgreSQL to start...${NC}"
sleep 5

# Enable extension
echo -e "${BLUE}Enabling pg_stat_statements extension...${NC}"
docker exec test-postgres psql -U postgres -c "CREATE EXTENSION pg_stat_statements;"

echo -e "${GREEN}✓ PostgreSQL ready!${NC}"
echo ""

# Load test data
echo -e "${BLUE}Loading test data (this will take 2-3 minutes)...${NC}"
echo "  - Creating 500K customers"
echo "  - Creating 10K products"
echo "  - Creating 1M orders"
echo "  - Creating 3M order items"
echo ""

docker exec -i test-postgres psql -U postgres < scripts/test-data.sql

echo -e "${GREEN}✓ Test data loaded!${NC}"
echo ""

# Run problematic queries multiple times
echo -e "${BLUE}Running workload queries (20 iterations)...${NC}"
for i in {1..20}; do
  echo -n "."
  docker exec test-postgres psql -U postgres -f - < scripts/test-queries.sql > /dev/null 2>&1
done
echo ""
echo -e "${GREEN}✓ Workload complete!${NC}"
echo ""

# Show what pg_stat_statements collected
echo -e "${BLUE}Query statistics in pg_stat_statements:${NC}"
docker exec test-postgres psql -U postgres -c "
SELECT 
    calls,
    ROUND(mean_exec_time::numeric, 2) as avg_ms,
    LEFT(query, 80) as query_snippet
FROM pg_stat_statements 
WHERE query NOT LIKE '%pg_%'
  AND query NOT LIKE '%pg_stat_statements%'
ORDER BY mean_exec_time DESC 
LIMIT 10;
" 2>/dev/null || echo "Stats will be collected after queries run"

echo ""
echo -e "${YELLOW}================================================${NC}"
echo -e "${YELLOW}Running PostgreSQL Query Health Advisor...${NC}"
echo -e "${YELLOW}================================================${NC}"
echo ""

# Run the advisor
./target/release/pg-query-advisor --database-url postgresql://postgres:postgres@localhost:5432/postgres

echo ""
echo -e "${GREEN}================================================${NC}"
echo -e "${GREEN}Test Complete!${NC}"
echo -e "${GREEN}================================================${NC}"
echo ""
echo "You should see recommendations for:"
echo "  ✓ Missing indexes on frequently queried columns"
echo "  ✓ Slow queries that need optimization"
echo "  ✓ Unused index (idx_products_created_at)"
echo "  ✓ Cost savings estimates"
echo ""
echo "To export JSON report:"
echo "  ./target/release/pg-query-advisor -d postgresql://postgres:postgres@localhost:5432/postgres --format json --output report.json"
echo ""
echo "To clean up:"
echo "  docker stop test-postgres && docker rm test-postgres"
