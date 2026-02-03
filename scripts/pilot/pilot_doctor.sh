#!/bin/bash
#
# Vaanyze Pilot Doctor
#
# Checks readiness for a pilot, validates prerequisites, and generates a report.
#
# Usage:
#   ./pilot_doctor.sh [OPTIONS]
#
# Options:
#   --force          Continue even if prerequisites fail
#   --output-dir     Directory for artifacts (default: pilot-artifacts/<timestamp>)
#   --quiet          Suppress console output, only write to file
#
# Environment:
#   DATABASE_URL         PostgreSQL connection string (required if Vaanyze not running)
#   VAANYZE_BASE_URL     Vaanyze HTTP endpoint (default: http://localhost:9090)
#
set -euo pipefail

# Colors (disabled if not a terminal)
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
FORCE=false
QUIET=false
OUTPUT_DIR=""
TIMESTAMP=$(date +%Y%m%d_%H%M%S)

# Parse arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        --force) FORCE=true; shift ;;
        --output-dir) OUTPUT_DIR="$2"; shift 2 ;;
        --quiet) QUIET=true; shift ;;
        -h|--help)
            echo "Usage: $0 [--force] [--output-dir DIR] [--quiet]"
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

# Logging functions
log() {
    [[ "$QUIET" == "false" ]] && echo -e "$@"
}

log_pass() {
    log "${GREEN}✓${NC} $1"
}

log_fail() {
    log "${RED}✗${NC} $1"
}

log_warn() {
    log "${YELLOW}⚠${NC} $1"
}

log_info() {
    log "${BLUE}ℹ${NC} $1"
}

log_header() {
    log ""
    log "${BOLD}$1${NC}"
    log "$(printf '=%.0s' {1..60})"
}

# Check for jq
HAS_JQ=true
if ! command -v jq &> /dev/null; then
    HAS_JQ=false
    log_warn "jq not found - using basic JSON parsing (install jq for better output)"
fi

# JSON extraction helper (works with or without jq)
json_get() {
    local json="$1"
    local key="$2"
    if [[ "$HAS_JQ" == "true" ]]; then
        echo "$json" | jq -r ".$key // empty" 2>/dev/null || echo ""
    else
        # Basic extraction using grep/sed (limited but functional)
        echo "$json" | grep -o "\"$key\":[^,}]*" | sed 's/.*://' | tr -d ' "' || echo ""
    fi
}

json_get_nested() {
    local json="$1"
    local path="$2"
    if [[ "$HAS_JQ" == "true" ]]; then
        echo "$json" | jq -r ".$path // empty" 2>/dev/null || echo ""
    else
        echo ""
    fi
}

# Initialize report
REPORT='{}'
CHECKS_PASSED=0
CHECKS_FAILED=0
CHECKS_WARNED=0

add_check() {
    local name="$1"
    local status="$2"
    local message="$3"
    local details="${4:-}"
    
    case "$status" in
        pass) CHECKS_PASSED=$((CHECKS_PASSED + 1)) ;;
        fail) CHECKS_FAILED=$((CHECKS_FAILED + 1)) ;;
        warn) CHECKS_WARNED=$((CHECKS_WARNED + 1)) ;;
    esac
    
    if [[ "$HAS_JQ" == "true" ]]; then
        REPORT=$(echo "$REPORT" | jq --arg name "$name" --arg status "$status" --arg msg "$message" --arg det "$details" \
            '.checks += [{"name": $name, "status": $status, "message": $msg, "details": $det}]')
    fi
}

# ============================================================================
# Check 1: Vaanyze Connectivity
# ============================================================================
log_header "1. Vaanyze Connectivity"

HEALTH_RESPONSE=""
VAANYZE_AVAILABLE=false

if HEALTH_RESPONSE=$(curl -sf "${VAANYZE_BASE_URL}/api/v1/health" 2>/dev/null); then
    VAANYZE_AVAILABLE=true
    log_pass "Vaanyze reachable at ${VAANYZE_BASE_URL}"
    
    # Extract health data
    if [[ "$HAS_JQ" == "true" ]]; then
        HEALTH_DATA=$(echo "$HEALTH_RESPONSE" | jq -r '.data // .')
        STATUS=$(echo "$HEALTH_DATA" | jq -r '.status // "unknown"')
        VERSION=$(echo "$HEALTH_DATA" | jq -r '.version // "unknown"')
        DB_CONNECTED=$(echo "$HEALTH_DATA" | jq -r '.database_connected // false')
        OPERATION_MODE=$(echo "$HEALTH_DATA" | jq -r '.operation_mode // "unknown"')
        READ_ONLY=$(echo "$HEALTH_DATA" | jq -r '.read_only // false')
        KILL_SWITCH=$(echo "$HEALTH_DATA" | jq -r '.kill_switch_active // false')
    else
        STATUS=$(json_get "$HEALTH_RESPONSE" "status")
        VERSION=$(json_get "$HEALTH_RESPONSE" "version")
        DB_CONNECTED=$(json_get "$HEALTH_RESPONSE" "database_connected")
        OPERATION_MODE=$(json_get "$HEALTH_RESPONSE" "operation_mode")
        READ_ONLY=$(json_get "$HEALTH_RESPONSE" "read_only")
        KILL_SWITCH=$(json_get "$HEALTH_RESPONSE" "kill_switch_active")
    fi
    
    log_info "  Version: ${VERSION}"
    log_info "  Status: ${STATUS}"
    log_info "  Operation Mode: ${OPERATION_MODE}"
    
    if [[ "$DB_CONNECTED" == "true" ]]; then
        log_pass "Database connected"
        add_check "database_connectivity" "pass" "Database is connected"
    else
        log_fail "Database not connected"
        add_check "database_connectivity" "fail" "Database is not connected"
    fi
    
    add_check "vaanyze_connectivity" "pass" "Vaanyze reachable" "$HEALTH_RESPONSE"
else
    log_fail "Cannot reach Vaanyze at ${VAANYZE_BASE_URL}"
    log_info "  Make sure Vaanyze is running: cargo run -- serve --port 9090"
    add_check "vaanyze_connectivity" "fail" "Cannot reach Vaanyze at ${VAANYZE_BASE_URL}"
fi

# ============================================================================
# Check 2: Environment Detection
# ============================================================================
log_header "2. Environment Detection"

ENV_RESPONSE=""
ENV_TYPE="unknown"
HAS_PG_STAT_STATEMENTS=false
CAN_CREATE_INDEX=false
RECOMMENDED_MODE=""

if [[ "$VAANYZE_AVAILABLE" == "true" ]]; then
    if ENV_RESPONSE=$(curl -sf "${VAANYZE_BASE_URL}/api/v1/system/environment" 2>/dev/null); then
        if [[ "$HAS_JQ" == "true" ]]; then
            ENV_DATA=$(echo "$ENV_RESPONSE" | jq -r '.data // .')
            ENV_TYPE=$(echo "$ENV_DATA" | jq -r '.environment // "unknown"')
            HAS_PG_STAT_STATEMENTS=$(echo "$ENV_DATA" | jq -r '.has_pg_stat_statements // false')
            CAN_CREATE_INDEX=$(echo "$ENV_DATA" | jq -r '.can_create_index // false')
            RECOMMENDED_MODE=$(echo "$ENV_DATA" | jq -r '.recommended_mode // "observe_only"')
            PG_VERSION=$(echo "$ENV_DATA" | jq -r '.pg_version // "unknown"')
            WARNINGS=$(echo "$ENV_DATA" | jq -r '.warnings[]? // empty')
        else
            ENV_TYPE=$(json_get "$ENV_RESPONSE" "environment")
            HAS_PG_STAT_STATEMENTS=$(json_get "$ENV_RESPONSE" "has_pg_stat_statements")
            CAN_CREATE_INDEX=$(json_get "$ENV_RESPONSE" "can_create_index")
            RECOMMENDED_MODE=$(json_get "$ENV_RESPONSE" "recommended_mode")
            PG_VERSION="unknown"
            WARNINGS=""
        fi
        
        log_info "  Environment: ${ENV_TYPE}"
        log_info "  PostgreSQL: ${PG_VERSION}"
        log_info "  Recommended mode: ${RECOMMENDED_MODE}"
        
        if [[ "$HAS_PG_STAT_STATEMENTS" == "true" ]]; then
            log_pass "pg_stat_statements enabled"
            add_check "pg_stat_statements" "pass" "Extension enabled and accessible"
        else
            log_fail "pg_stat_statements NOT available"
            log_info "  Run: CREATE EXTENSION IF NOT EXISTS pg_stat_statements;"
            add_check "pg_stat_statements" "fail" "Extension not available"
        fi
        
        if [[ "$CAN_CREATE_INDEX" == "true" ]]; then
            log_pass "CREATE/DROP INDEX permissions available"
            add_check "index_permissions" "pass" "User can create and drop indexes"
        else
            log_warn "No CREATE/DROP INDEX permissions"
            log_info "  Pilot will run in read-only mode"
            add_check "index_permissions" "warn" "No index modification permissions - read-only mode"
        fi
        
        # Warn about managed environments
        if [[ "$ENV_TYPE" != "SelfManaged" ]]; then
            log_warn "Detected managed environment: ${ENV_TYPE}"
            log_info "  Recommend starting in observe_only mode"
            add_check "managed_db" "warn" "Managed database detected: ${ENV_TYPE}"
        fi
        
        # Show warnings from environment detection
        if [[ -n "$WARNINGS" ]]; then
            log_warn "Environment warnings:"
            echo "$WARNINGS" | while read -r w; do
                [[ -n "$w" ]] && log_info "    - $w"
            done
        fi
        
        add_check "environment_detection" "pass" "Environment detected: ${ENV_TYPE}"
    else
        log_fail "Failed to detect environment"
        add_check "environment_detection" "fail" "Could not detect database environment"
    fi
else
    log_warn "Skipping environment detection (Vaanyze not available)"
    add_check "environment_detection" "warn" "Skipped - Vaanyze not available"
fi

# ============================================================================
# Check 3: Kill Switch Status
# ============================================================================
log_header "3. Safety Controls"

if [[ "$VAANYZE_AVAILABLE" == "true" ]]; then
    if [[ "$KILL_SWITCH" == "true" ]]; then
        log_warn "Kill switch is ACTIVE"
        log_info "  No apply/rollback operations will execute"
        log_info "  Disable with: curl -X POST ${VAANYZE_BASE_URL}/api/v1/system/kill-switch -d '{\"enabled\":false}'"
        add_check "kill_switch" "warn" "Kill switch is active - apply disabled"
    else
        log_pass "Kill switch is off"
        add_check "kill_switch" "pass" "Kill switch is not active"
    fi
    
    if [[ "$READ_ONLY" == "true" ]]; then
        log_info "System is in read-only mode"
        add_check "read_only_mode" "pass" "Read-only mode active (safe for observation)"
    else
        log_info "System is in apply-capable mode: ${OPERATION_MODE}"
        add_check "read_only_mode" "pass" "Apply-capable mode: ${OPERATION_MODE}"
    fi
else
    add_check "kill_switch" "warn" "Skipped - Vaanyze not available"
fi

# ============================================================================
# Check 4: Existing Indexes
# ============================================================================
log_header "4. Existing Vaanyze Indexes"

if [[ "$VAANYZE_AVAILABLE" == "true" ]]; then
    if INDEXES_RESPONSE=$(curl -sf "${VAANYZE_BASE_URL}/api/v1/system/indexes" 2>/dev/null); then
        if [[ "$HAS_JQ" == "true" ]]; then
            INDEXES_DATA=$(echo "$INDEXES_RESPONSE" | jq -r '.data // .')
            TOTAL_INDEXES=$(echo "$INDEXES_DATA" | jq -r '.total // 0')
            TOTAL_IDX_SCAN=$(echo "$INDEXES_DATA" | jq -r '.total_idx_scan // 0')
        else
            TOTAL_INDEXES=$(json_get "$INDEXES_RESPONSE" "total")
            TOTAL_IDX_SCAN=$(json_get "$INDEXES_RESPONSE" "total_idx_scan")
        fi
        
        if [[ "$TOTAL_INDEXES" -gt 0 ]]; then
            log_info "Found ${TOTAL_INDEXES} Vaanyze-managed index(es)"
            log_info "  Total idx_scan: ${TOTAL_IDX_SCAN}"
            add_check "existing_indexes" "pass" "${TOTAL_INDEXES} existing Vaanyze indexes"
        else
            log_pass "No existing Vaanyze indexes (clean slate)"
            add_check "existing_indexes" "pass" "No existing Vaanyze indexes"
        fi
    else
        log_warn "Could not check existing indexes"
        add_check "existing_indexes" "warn" "Could not check existing indexes"
    fi
else
    add_check "existing_indexes" "warn" "Skipped - Vaanyze not available"
fi

# ============================================================================
# Summary
# ============================================================================
log_header "Summary"

OVERALL_STATUS="pass"
if [[ $CHECKS_FAILED -gt 0 ]]; then
    OVERALL_STATUS="fail"
elif [[ $CHECKS_WARNED -gt 0 ]]; then
    OVERALL_STATUS="warn"
fi

log "  ${GREEN}Passed:${NC} ${CHECKS_PASSED}"
log "  ${YELLOW}Warnings:${NC} ${CHECKS_WARNED}"
log "  ${RED}Failed:${NC} ${CHECKS_FAILED}"
log ""

# Determine pilot readiness
PILOT_READY=true
PILOT_MODE="observe"

if [[ "$VAANYZE_AVAILABLE" != "true" ]]; then
    PILOT_READY=false
    log_fail "Pilot NOT ready: Vaanyze not reachable"
elif [[ "$HAS_PG_STAT_STATEMENTS" != "true" ]]; then
    PILOT_READY=false
    log_fail "Pilot NOT ready: pg_stat_statements required"
elif [[ "$CAN_CREATE_INDEX" != "true" ]]; then
    log_warn "Pilot can run in OBSERVE-ONLY mode"
    PILOT_MODE="observe_only"
else
    log_pass "Pilot ready for full validation"
    PILOT_MODE="full"
fi

# ============================================================================
# Generate Report
# ============================================================================
REPORT_FILE="${OUTPUT_DIR}/doctor.json"

if [[ "$HAS_JQ" == "true" ]]; then
    REPORT=$(echo "$REPORT" | jq \
        --arg ts "$(date -Iseconds)" \
        --arg vaanyze_url "$VAANYZE_BASE_URL" \
        --arg vaanyze_available "$VAANYZE_AVAILABLE" \
        --arg env_type "$ENV_TYPE" \
        --arg pg_stat "$HAS_PG_STAT_STATEMENTS" \
        --arg can_create "$CAN_CREATE_INDEX" \
        --arg rec_mode "$RECOMMENDED_MODE" \
        --arg pilot_ready "$PILOT_READY" \
        --arg pilot_mode "$PILOT_MODE" \
        --argjson passed "$CHECKS_PASSED" \
        --argjson warned "$CHECKS_WARNED" \
        --argjson failed "$CHECKS_FAILED" \
        --arg overall "$OVERALL_STATUS" \
        '. + {
            "timestamp": $ts,
            "vaanyze_url": $vaanyze_url,
            "vaanyze_available": ($vaanyze_available == "true"),
            "environment": $env_type,
            "has_pg_stat_statements": ($pg_stat == "true"),
            "can_create_index": ($can_create == "true"),
            "recommended_mode": $rec_mode,
            "pilot_ready": ($pilot_ready == "true"),
            "pilot_mode": $pilot_mode,
            "summary": {
                "passed": $passed,
                "warned": $warned,
                "failed": $failed,
                "overall": $overall
            }
        }')
    echo "$REPORT" | jq . > "$REPORT_FILE"
else
    # Basic JSON without jq
    cat > "$REPORT_FILE" << EOF
{
    "timestamp": "$(date -Iseconds)",
    "vaanyze_url": "${VAANYZE_BASE_URL}",
    "vaanyze_available": ${VAANYZE_AVAILABLE},
    "environment": "${ENV_TYPE}",
    "has_pg_stat_statements": ${HAS_PG_STAT_STATEMENTS},
    "can_create_index": ${CAN_CREATE_INDEX},
    "pilot_ready": ${PILOT_READY},
    "pilot_mode": "${PILOT_MODE}",
    "summary": {
        "passed": ${CHECKS_PASSED},
        "warned": ${CHECKS_WARNED},
        "failed": ${CHECKS_FAILED},
        "overall": "${OVERALL_STATUS}"
    }
}
EOF
fi

log ""
log "Report written to: ${REPORT_FILE}"

# Exit code
if [[ "$FORCE" == "true" ]]; then
    exit 0
elif [[ "$PILOT_READY" != "true" ]]; then
    exit 1
else
    exit 0
fi
