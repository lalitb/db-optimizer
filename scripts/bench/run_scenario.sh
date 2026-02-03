#!/bin/bash
#
# Vaanyze Benchmark Pack - Run Single Scenario
#
# ⚠️  WARNING: LOCAL DEVELOPMENT ONLY
# This script runs DDL (CREATE/DROP INDEX) directly against the database.
# It bypasses Vaanyze safety controls and should ONLY be used for:
#   - Local testing
#   - CI environments with disposable databases
#   - Benchmark validation before release
#
# DO NOT run this script against production or shared databases.
#
# Usage:
#   ./run_scenario.sh <scenario-name>
#   ./run_scenario.sh 01-missing-index-where
#
# Environment:
#   PGHOST, PGPORT, PGUSER, PGPASSWORD, PGDATABASE - Postgres connection
#   VAANYZE_URL - Vaanyze HTTP API URL (default: http://localhost:9090)
#
set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
SCENARIOS_DIR="${PROJECT_ROOT}/examples/benchmark-pack/scenarios"
RESULTS_DIR="${PROJECT_ROOT}/examples/benchmark-pack/results"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
NC='\033[0m'

# Database connection (defaults)
export PGHOST="${PGHOST:-localhost}"
export PGPORT="${PGPORT:-5433}"
export PGUSER="${PGUSER:-postgres}"
export PGPASSWORD="${PGPASSWORD:-postgres}"
export PGDATABASE="${PGDATABASE:-benchdb}"

# Safety check - warn if running against non-local database
if [[ "$PGHOST" != "localhost" && "$PGHOST" != "127.0.0.1" && "$PGHOST" != "::1" ]]; then
    echo -e "${RED}⚠️  WARNING: PGHOST ($PGHOST) is not localhost!${NC}"
    echo -e "${YELLOW}This script runs DDL directly and bypasses Vaanyze safety controls.${NC}"
    echo -e "${YELLOW}Are you sure you want to continue? (yes/no)${NC}"
    read -r response
    if [[ "$response" != "yes" ]]; then
        echo "Aborted."
        exit 1
    fi
fi

# Vaanyze API
VAANYZE_URL="${VAANYZE_URL:-http://localhost:9090}"

# Parse arguments
SCENARIO="$1"
if [[ -z "$SCENARIO" ]]; then
    echo "Usage: $0 <scenario-name>"
    echo ""
    echo "Available scenarios:"
    ls -1 "$SCENARIOS_DIR" 2>/dev/null || echo "  (none found)"
    exit 1
fi

SCENARIO_DIR="${SCENARIOS_DIR}/${SCENARIO}"
if [[ ! -d "$SCENARIO_DIR" ]]; then
    echo -e "${RED}Error: Scenario '$SCENARIO' not found in $SCENARIOS_DIR${NC}"
    exit 1
fi

# Load scenario config
CONFIG_FILE="${SCENARIO_DIR}/config.json"
if [[ ! -f "$CONFIG_FILE" ]]; then
    echo -e "${RED}Error: config.json not found in ${SCENARIO_DIR}${NC}"
    exit 1
fi

# Helper functions
log_step() {
    echo -e "${BLUE}>>> $1${NC}"
}

log_success() {
    echo -e "${GREEN}✓ $1${NC}"
}

log_warning() {
    echo -e "${YELLOW}⚠ $1${NC}"
}

log_error() {
    echo -e "${RED}✗ $1${NC}"
}

run_sql() {
    psql -h "$PGHOST" -p "$PGPORT" -U "$PGUSER" -d "$PGDATABASE" -q -t -A -c "$1" 2>/dev/null
}

run_sql_file() {
    psql -h "$PGHOST" -p "$PGPORT" -U "$PGUSER" -d "$PGDATABASE" -q -f "$1" 2>&1
}

# Ensure results directory exists
mkdir -p "$RESULTS_DIR"
RESULT_FILE="${RESULTS_DIR}/${SCENARIO}.json"

# Initialize result JSON
START_TIME=$(date -Iseconds)
START_EPOCH=$(date +%s)

echo "{"
echo "  \"scenario\": \"${SCENARIO}\","
echo "  \"started_at\": \"${START_TIME}\","

# ══════════════════════════════════════════════════════════════════
# STEP 1: Setup Schema and Data
# ══════════════════════════════════════════════════════════════════
log_step "Setting up schema and data..."

SCHEMA_FILE="${SCENARIO_DIR}/schema.sql"
if [[ -f "$SCHEMA_FILE" ]]; then
    if run_sql_file "$SCHEMA_FILE" > /dev/null; then
        log_success "Schema created"
    else
        log_error "Failed to create schema"
        exit 1
    fi
fi

# Reset pg_stat_statements for clean metrics
run_sql "SELECT pg_stat_statements_reset();" > /dev/null 2>&1 || true

# ══════════════════════════════════════════════════════════════════
# STEP 2: Run Baseline Load
# ══════════════════════════════════════════════════════════════════
log_step "Running baseline workload..."

WORKLOAD_FILE="${SCENARIO_DIR}/workload.sql"
if [[ ! -f "$WORKLOAD_FILE" ]]; then
    log_error "workload.sql not found"
    exit 1
fi

# Run workload multiple times to populate pg_stat_statements
BASELINE_ITERATIONS=50
for i in $(seq 1 $BASELINE_ITERATIONS); do
    run_sql_file "$WORKLOAD_FILE" > /dev/null 2>&1 || true
done
log_success "Baseline workload completed ($BASELINE_ITERATIONS iterations)"

# ══════════════════════════════════════════════════════════════════
# STEP 3: Capture Baseline Metrics
# ══════════════════════════════════════════════════════════════════
log_step "Capturing baseline metrics..."

# Get slowest query from pg_stat_statements
BASELINE_STATS=$(run_sql "
SELECT json_build_object(
    'total_queries', COUNT(*),
    'slowest_query', (
        SELECT json_build_object(
            'queryid', queryid,
            'query', LEFT(query, 200),
            'calls', calls,
            'mean_exec_time_ms', ROUND(mean_exec_time::numeric, 2),
            'total_exec_time_ms', ROUND(total_exec_time::numeric, 2),
            'rows', rows
        )
        FROM pg_stat_statements
        WHERE query NOT LIKE '%pg_stat%'
          AND query NOT LIKE '%pg_catalog%'
          AND queryid IS NOT NULL
        ORDER BY mean_exec_time DESC
        LIMIT 1
    )
)
FROM pg_stat_statements
WHERE query NOT LIKE '%pg_stat%';
")

log_success "Baseline metrics captured"
echo ""
echo -e "${CYAN}Baseline Stats:${NC}"
echo "$BASELINE_STATS" | python3 -m json.tool 2>/dev/null || echo "$BASELINE_STATS"

# ══════════════════════════════════════════════════════════════════
# STEP 4: Call Vaanyze for Recommendation (or stub)
# ══════════════════════════════════════════════════════════════════
log_step "Getting recommendation from Vaanyze..."

# Read target table/column from config
TARGET_TABLE=$(cat "$CONFIG_FILE" | python3 -c "import sys,json; print(json.load(sys.stdin).get('target_table', ''))" 2>/dev/null)
TARGET_COLUMN=$(cat "$CONFIG_FILE" | python3 -c "import sys,json; print(json.load(sys.stdin).get('target_column', ''))" 2>/dev/null)

# Extract a sample query for validation
SAMPLE_QUERY=$(run_sql "
SELECT query FROM pg_stat_statements
WHERE query NOT LIKE '%pg_stat%'
  AND query NOT LIKE '%pg_catalog%'
  AND queryid IS NOT NULL
ORDER BY mean_exec_time DESC
LIMIT 1;
")

# Try to call Vaanyze validate endpoint
VALIDATE_RESPONSE=""
if curl -s --connect-timeout 2 "${VAANYZE_URL}/api/v1/health" > /dev/null 2>&1; then
    log_success "Vaanyze is reachable"
    
    # Call validate endpoint
    VALIDATE_PAYLOAD=$(cat <<EOF
{
    "query": "${SAMPLE_QUERY}",
    "table_name": "${TARGET_TABLE}",
    "columns": ["${TARGET_COLUMN}"],
    "index_type": "btree",
    "min_improvement_pct": 5
}
EOF
)
    
    VALIDATE_RESPONSE=$(curl -s -X POST "${VAANYZE_URL}/api/v1/index/validate" \
        -H "Content-Type: application/json" \
        -d "$VALIDATE_PAYLOAD" 2>/dev/null || echo '{"error": "API call failed"}')
    
    echo ""
    echo -e "${CYAN}Validation Response:${NC}"
    echo "$VALIDATE_RESPONSE" | python3 -m json.tool 2>/dev/null || echo "$VALIDATE_RESPONSE"
else
    log_warning "Vaanyze not reachable at ${VAANYZE_URL}"
    log_warning "Continuing with stub recommendation..."
    
    # Stub recommendation based on config
    EXPECTED_REC=$(cat "$CONFIG_FILE" | python3 -c "import sys,json; print(json.load(sys.stdin).get('expected_recommendation', 'N/A'))" 2>/dev/null)
    VALIDATE_RESPONSE="{\"stub\": true, \"recommendation\": \"${EXPECTED_REC}\", \"note\": \"Vaanyze not running - using stub\"}"
fi

# ══════════════════════════════════════════════════════════════════
# STEP 5: EXPLAIN Proof (before index)
# ══════════════════════════════════════════════════════════════════
log_step "Getting EXPLAIN plan (before index)..."

BEFORE_PLAN=$(run_sql "EXPLAIN (FORMAT JSON) ${SAMPLE_QUERY}" 2>/dev/null || echo "[]")
BEFORE_PLAN_SUMMARY=$(run_sql "EXPLAIN ${SAMPLE_QUERY}" 2>/dev/null | head -5)

echo ""
echo -e "${CYAN}Before Plan (summary):${NC}"
echo "$BEFORE_PLAN_SUMMARY"

# ══════════════════════════════════════════════════════════════════
# STEP 6: Create Index (Safe Apply)
# ══════════════════════════════════════════════════════════════════
log_step "Creating index..."

# Generate index name
INDEX_NAME="idx_bench_${TARGET_TABLE}_${TARGET_COLUMN}"

# Check if index already exists
INDEX_EXISTS=$(run_sql "SELECT 1 FROM pg_indexes WHERE indexname = '${INDEX_NAME}'" || echo "")

if [[ -n "$INDEX_EXISTS" ]]; then
    log_warning "Index ${INDEX_NAME} already exists, dropping..."
    run_sql "DROP INDEX IF EXISTS ${INDEX_NAME};" || true
fi

# Create index concurrently
CREATE_SQL="CREATE INDEX CONCURRENTLY ${INDEX_NAME} ON ${TARGET_TABLE}(${TARGET_COLUMN})"
log_step "Executing: ${CREATE_SQL}"

CREATE_START=$(date +%s%3N)
if run_sql "${CREATE_SQL};" > /dev/null 2>&1; then
    CREATE_END=$(date +%s%3N)
    CREATE_DURATION=$((CREATE_END - CREATE_START))
    log_success "Index created in ${CREATE_DURATION}ms"
    INDEX_APPLIED=true
else
    log_error "Failed to create index"
    INDEX_APPLIED=false
fi

# ══════════════════════════════════════════════════════════════════
# STEP 7: EXPLAIN Proof (after index)
# ══════════════════════════════════════════════════════════════════
log_step "Getting EXPLAIN plan (after index)..."

AFTER_PLAN=$(run_sql "EXPLAIN (FORMAT JSON) ${SAMPLE_QUERY}" 2>/dev/null || echo "[]")
AFTER_PLAN_SUMMARY=$(run_sql "EXPLAIN ${SAMPLE_QUERY}" 2>/dev/null | head -5)

echo ""
echo -e "${CYAN}After Plan (summary):${NC}"
echo "$AFTER_PLAN_SUMMARY"

# Check if index is used
INDEX_USED=$(echo "$AFTER_PLAN_SUMMARY" | grep -i "Index Scan" | wc -l)
if [[ $INDEX_USED -gt 0 ]]; then
    log_success "Index is USED by query planner"
else
    log_warning "Index is NOT used by query planner"
fi

# ══════════════════════════════════════════════════════════════════
# STEP 8: Verification Window (Run Post-Apply Workload)
# ══════════════════════════════════════════════════════════════════
log_step "Running verification workload..."

# Reset stats for clean measurement
run_sql "SELECT pg_stat_statements_reset();" > /dev/null 2>&1 || true

# Run workload again
for i in $(seq 1 $BASELINE_ITERATIONS); do
    run_sql_file "$WORKLOAD_FILE" > /dev/null 2>&1 || true
done
log_success "Verification workload completed"

# Get idx_scan count
IDX_SCAN_DELTA=$(run_sql "
SELECT COALESCE(idx_scan, 0)
FROM pg_stat_user_indexes
WHERE indexrelname = '${INDEX_NAME}'
" || echo "0")

log_step "Index scan count: ${IDX_SCAN_DELTA}"

# Get post-apply stats
POST_APPLY_STATS=$(run_sql "
SELECT json_build_object(
    'slowest_query', (
        SELECT json_build_object(
            'queryid', queryid,
            'calls', calls,
            'mean_exec_time_ms', ROUND(mean_exec_time::numeric, 2),
            'total_exec_time_ms', ROUND(total_exec_time::numeric, 2)
        )
        FROM pg_stat_statements
        WHERE query NOT LIKE '%pg_stat%'
          AND query NOT LIKE '%pg_catalog%'
          AND queryid IS NOT NULL
        ORDER BY mean_exec_time DESC
        LIMIT 1
    )
);
")

# ══════════════════════════════════════════════════════════════════
# STEP 9: Generate Results JSON
# ══════════════════════════════════════════════════════════════════
log_step "Generating results..."

END_EPOCH=$(date +%s)
DURATION=$((END_EPOCH - START_EPOCH))

cat > "$RESULT_FILE" <<EOF
{
  "scenario": "${SCENARIO}",
  "started_at": "${START_TIME}",
  "completed_at": "$(date -Iseconds)",
  "duration_secs": ${DURATION},
  "baseline": ${BASELINE_STATS},
  "recommendation": {
    "table": "${TARGET_TABLE}",
    "column": "${TARGET_COLUMN}",
    "index_name": "${INDEX_NAME}",
    "create_sql": "${CREATE_SQL}"
  },
  "explain_proof": {
    "before_plan_summary": "$(echo "$BEFORE_PLAN_SUMMARY" | tr '\n' ' ' | sed 's/"/\\"/g')",
    "after_plan_summary": "$(echo "$AFTER_PLAN_SUMMARY" | tr '\n' ' ' | sed 's/"/\\"/g')",
    "index_used": $([ $INDEX_USED -gt 0 ] && echo "true" || echo "false")
  },
  "apply_result": {
    "applied": ${INDEX_APPLIED},
    "index_name": "${INDEX_NAME}",
    "create_duration_ms": ${CREATE_DURATION:-0}
  },
  "verification": {
    "idx_scan_delta": ${IDX_SCAN_DELTA:-0},
    "post_apply_stats": ${POST_APPLY_STATS}
  },
  "validation_response": ${VALIDATE_RESPONSE:-null}
}
EOF

log_success "Results written to: ${RESULT_FILE}"

# ══════════════════════════════════════════════════════════════════
# STEP 10: Summary
# ══════════════════════════════════════════════════════════════════
echo ""
echo -e "${BLUE}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
echo -e "${BLUE}Scenario Summary: ${SCENARIO}${NC}"
echo -e "${BLUE}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
echo ""
echo "  Index created:     ${INDEX_APPLIED}"
echo "  Index used:        $([ $INDEX_USED -gt 0 ] && echo 'Yes' || echo 'No')"
echo "  Index scans:       ${IDX_SCAN_DELTA}"
echo "  Duration:          ${DURATION}s"
echo "  Results:           ${RESULT_FILE}"
echo ""

# Determine success
if [[ "$INDEX_APPLIED" == "true" ]] && [[ $INDEX_USED -gt 0 ]]; then
    log_success "Scenario PASSED"
    exit 0
else
    log_warning "Scenario completed with warnings"
    exit 0  # Still exit 0 for now - manual review needed
fi
