#!/bin/bash
#
# Vaanyze Pilot Apply One
#
# Guided workflow to apply a single validated recommendation.
#
# Usage:
#   ./pilot_apply_one.sh [OPTIONS]
#
# Options:
#   --recommendation-id ID   Apply specific recommendation
#   --pick-top-validated     Automatically pick the top validated recommendation
#   --verification-minutes N  Verification window duration (default: 30)
#   --output-dir DIR         Directory for artifacts
#   --force                  Skip confirmations
#
# Environment:
#   VAANYZE_BASE_URL     Vaanyze HTTP endpoint (default: http://localhost:9090)
#
set -euo pipefail

# Colors
if [[ -t 1 ]]; then
    RED='\033[0;31m'
    GREEN='\033[0;32m'
    YELLOW='\033[1;33m'
    BLUE='\033[0;34m'
    BOLD='\033[1m'
    NC='\033[0m'
else
    RED='' GREEN='' YELLOW='' BLUE='' BOLD='' NC=''
fi

# Defaults
VAANYZE_BASE_URL="${VAANYZE_BASE_URL:-http://localhost:9090}"
RECOMMENDATION_ID=""
PICK_TOP=false
VERIFICATION_MINUTES=30
OUTPUT_DIR=""
FORCE=false
TIMESTAMP=$(date +%Y%m%d_%H%M%S)

# Parse arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        --recommendation-id) RECOMMENDATION_ID="$2"; shift 2 ;;
        --pick-top-validated) PICK_TOP=true; shift ;;
        --verification-minutes) VERIFICATION_MINUTES="$2"; shift 2 ;;
        --output-dir) OUTPUT_DIR="$2"; shift 2 ;;
        --force) FORCE=true; shift ;;
        -h|--help)
            echo "Usage: $0 [--recommendation-id ID | --pick-top-validated] [--verification-minutes N] [--force]"
            exit 0
            ;;
        *) echo "Unknown option: $1"; exit 1 ;;
    esac
done

# Set output directory
if [[ -z "$OUTPUT_DIR" ]]; then
    SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
    PROJECT_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
    OUTPUT_DIR="${PROJECT_ROOT}/pilot-artifacts/${TIMESTAMP}"
fi

mkdir -p "$OUTPUT_DIR"

# Check for jq
HAS_JQ=true
if ! command -v jq &> /dev/null; then
    echo "Error: jq is required for apply workflow"
    exit 1
fi

# Logging
log() {
    echo -e "[$(date '+%Y-%m-%d %H:%M:%S')] $*"
}

log_info() {
    log "${BLUE}INFO${NC} $*"
}

log_success() {
    log "${GREEN}OK${NC} $*"
}

log_warn() {
    log "${YELLOW}WARN${NC} $*"
}

log_error() {
    log "${RED}ERROR${NC} $*"
}

log_header() {
    echo ""
    echo -e "${BOLD}$1${NC}"
    echo "$(printf '=%.0s' {1..60})"
}

confirm() {
    if [[ "$FORCE" == "true" ]]; then
        return 0
    fi
    local msg="$1"
    read -p "$msg [y/N] " -n 1 -r
    echo
    [[ $REPLY =~ ^[Yy]$ ]]
}

# ============================================================================
# Pre-flight Checks
# ============================================================================

log_header "Pre-flight Checks"

# Check Vaanyze is available
log_info "Checking Vaanyze availability..."
HEALTH=$(curl -sf "${VAANYZE_BASE_URL}/api/v1/health" 2>/dev/null) || {
    log_error "Cannot reach Vaanyze at ${VAANYZE_BASE_URL}"
    exit 1
}

HEALTH_DATA=$(echo "$HEALTH" | jq -r '.data // .')
STATUS=$(echo "$HEALTH_DATA" | jq -r '.status')
KILL_SWITCH=$(echo "$HEALTH_DATA" | jq -r '.kill_switch_active')
READ_ONLY=$(echo "$HEALTH_DATA" | jq -r '.read_only')
OPERATION_MODE=$(echo "$HEALTH_DATA" | jq -r '.operation_mode')

log_success "Vaanyze is reachable (status: ${STATUS})"

# Check kill switch
if [[ "$KILL_SWITCH" == "true" ]]; then
    log_error "Kill switch is ACTIVE - apply operations are disabled"
    log_info "Disable with: curl -X POST ${VAANYZE_BASE_URL}/api/v1/system/kill-switch -d '{\"enabled\":false}'"
    
    if confirm "Would you like to disable the kill switch now?"; then
        KILL_RESULT=$(curl -sf -X POST "${VAANYZE_BASE_URL}/api/v1/system/kill-switch" \
            -H "Content-Type: application/json" \
            -d '{"enabled": false, "reason": "Pilot apply requested"}' 2>/dev/null) || {
            log_error "Failed to disable kill switch"
            exit 1
        }
        log_success "Kill switch disabled"
    else
        exit 1
    fi
fi

# Check operation mode
if [[ "$READ_ONLY" == "true" ]] || [[ "$OPERATION_MODE" == "observe_only" ]]; then
    log_warn "System is in read-only mode (${OPERATION_MODE})"
    
    if [[ "$FORCE" == "true" ]]; then
        log_info "Attempting to switch to safe_auto mode..."
        MODE_RESULT=$(curl -sf -X POST "${VAANYZE_BASE_URL}/api/v1/system/mode" \
            -H "Content-Type: application/json" \
            -d '{"mode": "safe_auto"}' 2>/dev/null) || {
            log_error "Failed to change mode"
            exit 1
        }
        log_success "Mode changed to safe_auto"
    else
        log_error "Cannot apply in read-only mode. Use --force to switch mode, or run:"
        log_info "  curl -X POST ${VAANYZE_BASE_URL}/api/v1/system/mode -d '{\"mode\":\"safe_auto\"}'"
        exit 1
    fi
fi

log_success "System ready for apply operations"

# ============================================================================
# Select Recommendation
# ============================================================================

log_header "Select Recommendation"

if [[ -n "$RECOMMENDATION_ID" ]]; then
    log_info "Using specified recommendation: ${RECOMMENDATION_ID}"
elif [[ "$PICK_TOP" == "true" ]]; then
    log_info "Picking top validated recommendation..."
    
    # Get audit log to find validated recommendations
    AUDIT=$(curl -sf "${VAANYZE_BASE_URL}/api/v1/index/audit" 2>/dev/null) || {
        log_error "Failed to fetch audit log"
        exit 1
    }
    
    # Find the most recent VALIDATED recommendation
    AUDIT_DATA=$(echo "$AUDIT" | jq -r '.data // .')
    
    # Look for validation_succeeded events
    VALIDATED_IDS=$(echo "$AUDIT_DATA" | jq -r '.events[]? | select(.event_type == "validation_succeeded") | .message' | 
        grep -oE 'rec_[a-f0-9-]+|[a-f0-9-]{36}' | head -5)
    
    if [[ -z "$VALIDATED_IDS" ]]; then
        log_error "No validated recommendations found in audit log"
        log_info "Run validation first with: curl -X POST ${VAANYZE_BASE_URL}/api/v1/index/validate ..."
        exit 1
    fi
    
    RECOMMENDATION_ID=$(echo "$VALIDATED_IDS" | head -1)
    log_info "Selected recommendation: ${RECOMMENDATION_ID}"
else
    log_error "Must specify --recommendation-id or --pick-top-validated"
    exit 1
fi

# ============================================================================
# Fetch Recommendation Details
# ============================================================================

log_header "Recommendation Details"

# Get status of this recommendation
STATUS_RESPONSE=$(curl -sf "${VAANYZE_BASE_URL}/api/v1/index/status/${RECOMMENDATION_ID}" 2>/dev/null) || {
    log_error "Recommendation ${RECOMMENDATION_ID} not found"
    log_info "Check that the recommendation has been validated first"
    exit 1
}

STATUS_DATA=$(echo "$STATUS_RESPONSE" | jq -r '.data // .')
CURRENT_STATE=$(echo "$STATUS_DATA" | jq -r '.state')
TABLE_NAME=$(echo "$STATUS_DATA" | jq -r '.table_name')
COLUMNS=$(echo "$STATUS_DATA" | jq -r '.columns | join(", ")')
INDEX_NAME=$(echo "$STATUS_DATA" | jq -r '.index_name // "not yet created"')
VALIDATION=$(echo "$STATUS_DATA" | jq -r '.validation_result // {}')

log_info "  ID: ${RECOMMENDATION_ID}"
log_info "  State: ${CURRENT_STATE}"
log_info "  Table: ${TABLE_NAME}"
log_info "  Columns: ${COLUMNS}"

if [[ "$CURRENT_STATE" != "VALIDATED" ]]; then
    log_error "Recommendation is in state '${CURRENT_STATE}', must be 'VALIDATED' to apply"
    
    if [[ "$CURRENT_STATE" == "APPLIED" ]] || [[ "$CURRENT_STATE" == "VERIFYING" ]] || [[ "$CURRENT_STATE" == "STABLE" ]]; then
        log_info "This recommendation has already been applied (index: ${INDEX_NAME})"
    fi
    exit 1
fi

# Show validation proof
VALIDATED=$(echo "$VALIDATION" | jq -r '.validated // false')
COST_IMPROVEMENT=$(echo "$VALIDATION" | jq -r '.cost_improvement_pct // 0')
INDEX_USED=$(echo "$VALIDATION" | jq -r '.index_used // false')
REASON=$(echo "$VALIDATION" | jq -r '.reason // "unknown"')

log_info ""
log_info "Validation Proof:"
log_info "  Validated: ${VALIDATED}"
log_info "  Index used: ${INDEX_USED}"
log_info "  Cost improvement: ${COST_IMPROVEMENT}%"
log_info "  Reason: ${REASON}"

# Save before state
BEFORE_STATE_FILE="${OUTPUT_DIR}/apply_${RECOMMENDATION_ID}_before.json"
echo "$STATUS_DATA" | jq . > "$BEFORE_STATE_FILE"

# ============================================================================
# Confirm and Apply
# ============================================================================

log_header "Apply Confirmation"

# Show what will happen
cat << EOF

The following operation will be performed:

  CREATE INDEX CONCURRENTLY vaanyze_idx_${TABLE_NAME}_... ON ${TABLE_NAME} (${COLUMNS})

This is a non-blocking operation that will:
  1. Create the index without locking the table for writes
  2. Start a ${VERIFICATION_MINUTES}-minute verification window
  3. Monitor idx_scan and query performance
  4. Automatically rollback if no improvement detected

EOF

if ! confirm "Proceed with apply?"; then
    log_info "Apply cancelled"
    exit 0
fi

# ============================================================================
# Execute Apply
# ============================================================================

log_header "Executing Apply"

log_info "Sending apply request..."

APPLY_RESPONSE=$(curl -sf -X POST "${VAANYZE_BASE_URL}/api/v1/index/apply" \
    -H "Content-Type: application/json" \
    -d "{
        \"recommendation_id\": \"${RECOMMENDATION_ID}\",
        \"triggered_by\": \"pilot_apply_script\",
        \"verification_minutes\": ${VERIFICATION_MINUTES}
    }" 2>/dev/null) || {
    log_error "Apply request failed"
    exit 1
}

APPLY_DATA=$(echo "$APPLY_RESPONSE" | jq -r '.data // .')
APPLY_STATE=$(echo "$APPLY_DATA" | jq -r '.state')
APPLIED_INDEX=$(echo "$APPLY_DATA" | jq -r '.index_name')
VERIFICATION_UNTIL=$(echo "$APPLY_DATA" | jq -r '.verification_until')

if [[ "$APPLY_STATE" == "VERIFYING" ]] || [[ "$APPLY_STATE" == "APPLIED" ]]; then
    log_success "Index created successfully!"
    log_info "  Index: ${APPLIED_INDEX}"
    log_info "  State: ${APPLY_STATE}"
    log_info "  Verification until: ${VERIFICATION_UNTIL}"
else
    log_error "Unexpected state after apply: ${APPLY_STATE}"
    echo "$APPLY_RESPONSE" | jq .
    exit 1
fi

# ============================================================================
# Monitor Verification
# ============================================================================

log_header "Monitoring Verification"

log_info "Polling status every 30 seconds during verification window..."
log_info "Press Ctrl+C to stop monitoring (apply will continue in background)"
log_info ""

POLL_INTERVAL=30
POLLS=0
FINAL_STATE=""

while true; do
    POLLS=$((POLLS + 1))
    
    STATUS_RESPONSE=$(curl -sf "${VAANYZE_BASE_URL}/api/v1/index/status/${RECOMMENDATION_ID}" 2>/dev/null) || {
        log_warn "Failed to fetch status (attempt ${POLLS})"
        sleep $POLL_INTERVAL
        continue
    }
    
    STATUS_DATA=$(echo "$STATUS_RESPONSE" | jq -r '.data // .')
    CURRENT_STATE=$(echo "$STATUS_DATA" | jq -r '.state')
    
    case "$CURRENT_STATE" in
        "VERIFYING")
            # Get idx_scan if available
            IDX_SCAN=$(curl -sf "${VAANYZE_BASE_URL}/api/v1/system/indexes" 2>/dev/null | \
                jq -r ".data.indexes[]? | select(.name == \"$APPLIED_INDEX\") | .idx_scan" 2>/dev/null || echo "N/A")
            log_info "[Poll ${POLLS}] State: ${CURRENT_STATE} | idx_scan: ${IDX_SCAN}"
            ;;
        "STABLE")
            log_success "Verification PASSED - Index is stable and in use"
            FINAL_STATE="STABLE"
            break
            ;;
        "ROLLED_BACK")
            log_warn "Index was ROLLED BACK during verification"
            FINAL_STATE="ROLLED_BACK"
            break
            ;;
        "FAILED")
            log_error "Apply FAILED"
            FINAL_STATE="FAILED"
            break
            ;;
        *)
            log_info "[Poll ${POLLS}] State: ${CURRENT_STATE}"
            ;;
    esac
    
    # Check if verification window has passed
    if [[ -n "$VERIFICATION_UNTIL" ]]; then
        VERIFICATION_END=$(date -j -f "%Y-%m-%dT%H:%M:%S" "${VERIFICATION_UNTIL%Z*}" +%s 2>/dev/null || \
            date -d "$VERIFICATION_UNTIL" +%s 2>/dev/null || echo "0")
        NOW=$(date +%s)
        
        if [[ $NOW -gt $VERIFICATION_END ]]; then
            log_info "Verification window has passed"
            break
        fi
    fi
    
    sleep $POLL_INTERVAL
done

# ============================================================================
# Generate Report
# ============================================================================

log_header "Generating Report"

# Fetch final state
FINAL_STATUS=$(curl -sf "${VAANYZE_BASE_URL}/api/v1/index/status/${RECOMMENDATION_ID}" 2>/dev/null) || {
    log_warn "Failed to fetch final status"
}
FINAL_STATUS_DATA=$(echo "$FINAL_STATUS" | jq -r '.data // {}')

# Fetch audit log for this recommendation
AUDIT_EXCERPT=$(curl -sf "${VAANYZE_BASE_URL}/api/v1/index/audit" 2>/dev/null | \
    jq ".data.events | map(select(.message | contains(\"$RECOMMENDATION_ID\") or contains(\"$APPLIED_INDEX\")))" 2>/dev/null || echo "[]")

# Get index stats
INDEX_STATS=$(curl -sf "${VAANYZE_BASE_URL}/api/v1/system/indexes" 2>/dev/null | \
    jq ".data.indexes[]? | select(.name == \"$APPLIED_INDEX\")" 2>/dev/null || echo "{}")

# Build report
REPORT_FILE="${OUTPUT_DIR}/apply_${RECOMMENDATION_ID}.json"

jq -n \
    --arg rec_id "$RECOMMENDATION_ID" \
    --arg table "$TABLE_NAME" \
    --arg columns "$COLUMNS" \
    --arg index_name "$APPLIED_INDEX" \
    --arg final_state "${FINAL_STATE:-$CURRENT_STATE}" \
    --argjson before "$(cat "$BEFORE_STATE_FILE")" \
    --argjson after "$FINAL_STATUS_DATA" \
    --argjson audit "$AUDIT_EXCERPT" \
    --argjson index_stats "$INDEX_STATS" \
    --argjson verification_minutes "$VERIFICATION_MINUTES" \
    '{
        "recommendation_id": $rec_id,
        "table": $table,
        "columns": $columns,
        "index_name": $index_name,
        "final_state": $final_state,
        "verification_minutes": $verification_minutes,
        "before_apply": $before,
        "after_apply": $after,
        "index_stats": $index_stats,
        "audit_log_excerpt": $audit,
        "generated_at": (now | todate)
    }' > "$REPORT_FILE"

log_success "Report written to: ${REPORT_FILE}"

# ============================================================================
# Summary
# ============================================================================

log_header "Summary"

FINAL_STATE="${FINAL_STATE:-$CURRENT_STATE}"

case "$FINAL_STATE" in
    "STABLE")
        log_success "✓ Index applied and verified successfully"
        log_info "  Index: ${APPLIED_INDEX}"
        log_info "  The index is stable and being used by queries"
        ;;
    "ROLLED_BACK")
        log_warn "⚠ Index was rolled back"
        log_info "  The index did not show expected improvements"
        log_info "  This is the safe behavior - no harm was done"
        ;;
    "FAILED")
        log_error "✗ Apply failed"
        log_info "  Check the audit log for details"
        ;;
    "VERIFYING")
        log_info "⏳ Still in verification window"
        log_info "  The index will be automatically verified"
        log_info "  Check back later or run: curl ${VAANYZE_BASE_URL}/api/v1/index/status/${RECOMMENDATION_ID}"
        ;;
    *)
        log_info "Final state: ${FINAL_STATE}"
        ;;
esac

log_info ""
log_info "Full report: ${REPORT_FILE}"

exit 0
