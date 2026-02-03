#!/bin/bash
#
# Vaanyze Benchmark Pack - Run All Scenarios
#
# Usage:
#   ./run_all.sh
#   ./run_all.sh --skip-failing
#
set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SCENARIOS_DIR="${SCRIPT_DIR}/../examples/benchmark-pack/scenarios"
RESULTS_DIR="${SCRIPT_DIR}/../examples/benchmark-pack/results"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

# Options
SKIP_FAILING=false
if [[ "$1" == "--skip-failing" ]]; then
    SKIP_FAILING=true
fi

echo -e "${BLUE}╔════════════════════════════════════════════════════════════╗${NC}"
echo -e "${BLUE}║          Vaanyze Benchmark Pack - Full Suite               ║${NC}"
echo -e "${BLUE}╚════════════════════════════════════════════════════════════╝${NC}"
echo ""

# Ensure results directory exists
mkdir -p "$RESULTS_DIR"

# List all scenarios
SCENARIOS=(
    "01-missing-index-where"
    "02-composite-index"
    "03-join-path"
    "04-order-by-limit"
    "05-write-heavy-rollback"
    "06-wal-pressure-negative"
)

TOTAL=${#SCENARIOS[@]}
PASSED=0
FAILED=0
SKIPPED=0

# Summary file
SUMMARY_FILE="${RESULTS_DIR}/summary.json"
echo "{\"started_at\": \"$(date -Iseconds)\", \"scenarios\": [" > "$SUMMARY_FILE"

START_TIME=$(date +%s)

for i in "${!SCENARIOS[@]}"; do
    SCENARIO="${SCENARIOS[$i]}"
    SCENARIO_NUM=$((i + 1))
    
    echo ""
    echo -e "${BLUE}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
    echo -e "${BLUE}[$SCENARIO_NUM/$TOTAL] Running: ${SCENARIO}${NC}"
    echo -e "${BLUE}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
    
    # Run scenario
    if "$SCRIPT_DIR/run_scenario.sh" "$SCENARIO"; then
        echo -e "${GREEN}✓ PASSED: ${SCENARIO}${NC}"
        PASSED=$((PASSED + 1))
        STATUS="passed"
    else
        if [[ "$SKIP_FAILING" == "true" ]]; then
            echo -e "${YELLOW}⚠ SKIPPED (failed): ${SCENARIO}${NC}"
            SKIPPED=$((SKIPPED + 1))
            STATUS="skipped"
        else
            echo -e "${RED}✗ FAILED: ${SCENARIO}${NC}"
            FAILED=$((FAILED + 1))
            STATUS="failed"
        fi
    fi
    
    # Add to summary
    if [[ $i -gt 0 ]]; then
        echo "," >> "$SUMMARY_FILE"
    fi
    echo "  {\"name\": \"${SCENARIO}\", \"status\": \"${STATUS}\"}" >> "$SUMMARY_FILE"
done

END_TIME=$(date +%s)
DURATION=$((END_TIME - START_TIME))

# Close summary JSON
echo "" >> "$SUMMARY_FILE"
echo "], \"completed_at\": \"$(date -Iseconds)\", \"duration_secs\": ${DURATION}, \"summary\": {\"total\": ${TOTAL}, \"passed\": ${PASSED}, \"failed\": ${FAILED}, \"skipped\": ${SKIPPED}}}" >> "$SUMMARY_FILE"

# Print summary
echo ""
echo -e "${BLUE}╔════════════════════════════════════════════════════════════╗${NC}"
echo -e "${BLUE}║                    Benchmark Summary                       ║${NC}"
echo -e "${BLUE}╚════════════════════════════════════════════════════════════╝${NC}"
echo ""
echo -e "  Total scenarios:  ${TOTAL}"
echo -e "  ${GREEN}Passed:           ${PASSED}${NC}"
echo -e "  ${RED}Failed:           ${FAILED}${NC}"
echo -e "  ${YELLOW}Skipped:          ${SKIPPED}${NC}"
echo ""
echo -e "  Duration:         ${DURATION}s"
echo -e "  Results:          ${RESULTS_DIR}/"
echo -e "  Summary:          ${SUMMARY_FILE}"
echo ""

# Exit with appropriate code
if [[ $FAILED -gt 0 && "$SKIP_FAILING" != "true" ]]; then
    echo -e "${RED}Some scenarios failed!${NC}"
    exit 1
else
    echo -e "${GREEN}All scenarios completed!${NC}"
    exit 0
fi
