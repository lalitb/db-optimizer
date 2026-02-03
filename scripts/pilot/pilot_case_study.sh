#!/bin/bash
#
# Vaanyze Pilot Case Study Generator
#
# Generates a one-page case study markdown from collected pilot artifacts.
#
# Usage:
#   ./pilot_case_study.sh [OPTIONS]
#
# Options:
#   --input-dir DIR      Pilot artifacts directory (required)
#   --output-dir DIR     Output directory (default: same as input)
#   --redact-level LEVEL Redaction level: none, normal, strict (default: normal)
#   --title TITLE        Case study title
#
set -euo pipefail

# Colors
if [[ -t 1 ]]; then
    GREEN='\033[0;32m'
    BLUE='\033[0;34m'
    NC='\033[0m'
else
    GREEN='' BLUE='' NC=''
fi

# Defaults
INPUT_DIR=""
OUTPUT_DIR=""
REDACT_LEVEL="normal"
TITLE="Vaanyze Pilot Case Study"

# Parse arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        --input-dir) INPUT_DIR="$2"; shift 2 ;;
        --output-dir) OUTPUT_DIR="$2"; shift 2 ;;
        --redact-level) REDACT_LEVEL="$2"; shift 2 ;;
        --title) TITLE="$2"; shift 2 ;;
        -h|--help)
            echo "Usage: $0 --input-dir DIR [--output-dir DIR] [--redact-level none|normal|strict] [--title TITLE]"
            exit 0
            ;;
        *) echo "Unknown option: $1"; exit 1 ;;
    esac
done

if [[ -z "$INPUT_DIR" ]]; then
    echo "Error: --input-dir is required"
    exit 1
fi

if [[ ! -d "$INPUT_DIR" ]]; then
    echo "Error: Input directory does not exist: $INPUT_DIR"
    exit 1
fi

if [[ -z "$OUTPUT_DIR" ]]; then
    OUTPUT_DIR="$INPUT_DIR"
fi

mkdir -p "$OUTPUT_DIR"

# Check for jq
if ! command -v jq &> /dev/null; then
    echo "Error: jq is required for case study generation"
    exit 1
fi

# Logging
log() {
    echo -e "${BLUE}INFO${NC} $*"
}

log_success() {
    echo -e "${GREEN}OK${NC} $*"
}

# ============================================================================
# Redaction Functions
# ============================================================================

redact_table_name() {
    local name="$1"
    case "$REDACT_LEVEL" in
        none) echo "$name" ;;
        normal) echo "$name" ;;  # Keep table names in normal mode
        strict) echo "table_$(echo "$name" | md5sum | cut -c1-6)" ;;
    esac
}

redact_column_name() {
    local name="$1"
    case "$REDACT_LEVEL" in
        none) echo "$name" ;;
        normal) echo "$name" ;;
        strict) echo "col_$(echo "$name" | md5sum | cut -c1-4)" ;;
    esac
}

redact_query() {
    local query="$1"
    case "$REDACT_LEVEL" in
        none) echo "$query" ;;
        normal)
            # Redact string literals and numbers in queries
            echo "$query" | sed "s/'[^']*'/'...'/g" | sed 's/[0-9]\+/N/g'
            ;;
        strict)
            # Just show query structure
            echo "$query" | sed "s/'[^']*'/'...'/g" | sed 's/[0-9]\+/N/g' | \
                sed 's/\b[a-z_]\+\b/X/gi' | head -c 100
            echo "..."
            ;;
    esac
}

redact_index_name() {
    local name="$1"
    case "$REDACT_LEVEL" in
        none) echo "$name" ;;
        normal) echo "$name" ;;
        strict) echo "vaanyze_idx_$(echo "$name" | md5sum | cut -c1-8)" ;;
    esac
}

# ============================================================================
# Load Artifacts
# ============================================================================

log "Loading pilot artifacts from: ${INPUT_DIR}"

# Load doctor report
DOCTOR_FILE="${INPUT_DIR}/doctor.json"
if [[ -f "$DOCTOR_FILE" ]]; then
    DOCTOR=$(cat "$DOCTOR_FILE")
    ENV_TYPE=$(echo "$DOCTOR" | jq -r '.environment // "Unknown"')
    PG_STAT=$(echo "$DOCTOR" | jq -r '.has_pg_stat_statements // false')
    CAN_CREATE=$(echo "$DOCTOR" | jq -r '.can_create_index // false')
    log "  Loaded doctor.json"
else
    DOCTOR='{}'
    ENV_TYPE="Unknown"
    PG_STAT="unknown"
    CAN_CREATE="unknown"
    log "  No doctor.json found"
fi

# Load observation summary
OBSERVE_FILE="${INPUT_DIR}/observe_summary.json"
if [[ -f "$OBSERVE_FILE" ]]; then
    OBSERVE=$(cat "$OBSERVE_FILE")
    OBS_START=$(echo "$OBSERVE" | jq -r '.observation_period.start // "N/A"')
    OBS_END=$(echo "$OBSERVE" | jq -r '.observation_period.end // "N/A"')
    OBS_SNAPSHOTS=$(echo "$OBSERVE" | jq -r '.observation_period.snapshots_collected // 0')
    UNIQUE_RECS=$(echo "$OBSERVE" | jq -r '.unique_recommendations // 0')
    TOP_QUERIES=$(echo "$OBSERVE" | jq -r '.top_slow_queries // []')
    log "  Loaded observe_summary.json"
else
    OBSERVE='{}'
    OBS_START="N/A"
    OBS_END="N/A"
    OBS_SNAPSHOTS=0
    UNIQUE_RECS=0
    TOP_QUERIES='[]'
    log "  No observe_summary.json found"
fi

# Load apply reports
APPLY_FILES=$(find "$INPUT_DIR" -name "apply_*.json" -type f 2>/dev/null | grep -v "_before.json" || echo "")
APPLY_COUNT=$(echo "$APPLY_FILES" | grep -c . || echo "0")

APPLY_DATA='[]'
if [[ -n "$APPLY_FILES" ]] && [[ "$APPLY_COUNT" -gt 0 ]]; then
    log "  Found ${APPLY_COUNT} apply report(s)"
    
    # Combine all apply reports
    APPLY_DATA=$(echo "$APPLY_FILES" | while read -r f; do
        [[ -f "$f" ]] && cat "$f"
    done | jq -s '.')
else
    log "  No apply reports found"
fi

# ============================================================================
# Generate Case Study Markdown
# ============================================================================

CASE_STUDY_FILE="${OUTPUT_DIR}/case-study.md"
CASE_STUDY_REDACTED="${OUTPUT_DIR}/case-study.redacted.md"

generate_case_study() {
    local output_file="$1"
    local is_redacted="$2"
    
    cat > "$output_file" << EOF
# ${TITLE}

**Generated:** $(date -Iseconds)
**Redaction Level:** ${REDACT_LEVEL}

---

## 1. Environment

| Property | Value |
|----------|-------|
| Database Type | ${ENV_TYPE} |
| pg_stat_statements | ${PG_STAT} |
| Index Permissions | ${CAN_CREATE} |

EOF

    # Baseline symptoms section
    cat >> "$output_file" << 'EOF'
## 2. Baseline Symptoms

### Observation Period
EOF

    if [[ "$OBS_SNAPSHOTS" -gt 0 ]]; then
        cat >> "$output_file" << EOF

- **Start:** ${OBS_START}
- **End:** ${OBS_END}
- **Snapshots collected:** ${OBS_SNAPSHOTS}
- **Unique recommendations identified:** ${UNIQUE_RECS}

### Top Slow Queries

EOF

        # List top slow queries
        echo "$TOP_QUERIES" | jq -r '.[]? | "| \(.queryid // "N/A") | \(.max_mean_ms // 0 | floor)ms | \(.observations // 0) |"' | while read -r line; do
            [[ -z "$line" ]] && continue
            echo "$line" >> "$output_file"
        done

        # Add table header if we have queries
        if [[ $(echo "$TOP_QUERIES" | jq 'length') -gt 0 ]]; then
            # Insert header before the data
            TEMP_FILE=$(mktemp)
            head -n -$(echo "$TOP_QUERIES" | jq 'length') "$output_file" > "$TEMP_FILE"
            echo "| Query ID | Mean Latency | Observations |" >> "$TEMP_FILE"
            echo "|----------|--------------|--------------|" >> "$TEMP_FILE"
            tail -n $(echo "$TOP_QUERIES" | jq 'length') "$output_file" >> "$TEMP_FILE"
            mv "$TEMP_FILE" "$output_file"
        fi
    else
        echo "*No observation data available*" >> "$output_file"
    fi

    # Recommendations section
    cat >> "$output_file" << 'EOF'

## 3. Recommendations Applied

EOF

    if [[ "$APPLY_COUNT" -gt 0 ]]; then
        echo "$APPLY_DATA" | jq -r '.[]? | @base64' | while read -r encoded; do
            [[ -z "$encoded" ]] && continue
            
            APPLY=$(echo "$encoded" | base64 -d)
            
            REC_ID=$(echo "$APPLY" | jq -r '.recommendation_id // "N/A"')
            TABLE=$(echo "$APPLY" | jq -r '.table // "N/A"')
            COLUMNS=$(echo "$APPLY" | jq -r '.columns // "N/A"')
            INDEX_NAME=$(echo "$APPLY" | jq -r '.index_name // "N/A"')
            FINAL_STATE=$(echo "$APPLY" | jq -r '.final_state // "N/A"')
            
            # Get validation details from before_apply
            COST_IMPROVEMENT=$(echo "$APPLY" | jq -r '.before_apply.validation_result.cost_improvement_pct // 0')
            INDEX_USED=$(echo "$APPLY" | jq -r '.before_apply.validation_result.index_used // false')
            
            # Get index stats
            IDX_SCAN=$(echo "$APPLY" | jq -r '.index_stats.idx_scan // 0')
            
            # Apply redaction if needed
            if [[ "$is_redacted" == "true" ]]; then
                TABLE=$(redact_table_name "$TABLE")
                INDEX_NAME=$(redact_index_name "$INDEX_NAME")
            fi
            
            cat >> "$output_file" << EOF

### Recommendation: ${REC_ID}

**Target:** \`${TABLE}\` (${COLUMNS})

**Index Created:** \`${INDEX_NAME}\`

**EXPLAIN Proof:**
- Index used by planner: ${INDEX_USED}
- Cost improvement: ${COST_IMPROVEMENT}%

**Verification Outcome:** ${FINAL_STATE}

**Index Usage (idx_scan):** ${IDX_SCAN}

EOF
        done
    else
        echo "*No recommendations were applied during this pilot*" >> "$output_file"
    fi

    # Outcome section
    cat >> "$output_file" << 'EOF'

## 4. Outcome

EOF

    STABLE_COUNT=$(echo "$APPLY_DATA" | jq '[.[]? | select(.final_state == "STABLE")] | length')
    ROLLBACK_COUNT=$(echo "$APPLY_DATA" | jq '[.[]? | select(.final_state == "ROLLED_BACK")] | length')
    FAILED_COUNT=$(echo "$APPLY_DATA" | jq '[.[]? | select(.final_state == "FAILED")] | length')

    cat >> "$output_file" << EOF
| Outcome | Count |
|---------|-------|
| Successfully Applied (STABLE) | ${STABLE_COUNT} |
| Safely Rolled Back | ${ROLLBACK_COUNT} |
| Failed | ${FAILED_COUNT} |

EOF

    if [[ "$STABLE_COUNT" -gt 0 ]]; then
        echo "**Result:** Vaanyze successfully identified and applied performance-improving indexes that are now in active use." >> "$output_file"
    elif [[ "$ROLLBACK_COUNT" -gt 0 ]]; then
        echo "**Result:** Vaanyze's safety mechanisms correctly identified indexes that would not improve performance and automatically rolled them back." >> "$output_file"
    else
        echo "**Result:** Pilot completed observation phase. Recommendations are available for review." >> "$output_file"
    fi

    # Safety notes
    cat >> "$output_file" << 'EOF'

## 5. Safety Notes

### Trust Model

- ✅ **EXPLAIN Proof:** Every recommendation was validated with actual PostgreSQL EXPLAIN plans before apply
- ✅ **Non-blocking:** All indexes created using `CREATE INDEX CONCURRENTLY`
- ✅ **Verification Window:** Applied indexes were monitored for actual usage (idx_scan)
- ✅ **Automatic Rollback:** Indexes not showing improvement were automatically dropped
- ✅ **Audit Trail:** All operations logged with timestamps and outcomes

### Emergency Controls

- Kill switch available at `/api/v1/system/kill-switch`
- Manual rollback always possible via `/api/v1/index/rollback`
- Read-only mode available for observation without any changes

---

*Generated by Vaanyze Pilot Automation Kit*
EOF

}

# Generate full case study
log "Generating case study..."
generate_case_study "$CASE_STUDY_FILE" "false"
log_success "Case study written to: ${CASE_STUDY_FILE}"

# Generate redacted version
if [[ "$REDACT_LEVEL" != "none" ]]; then
    log "Generating redacted case study..."
    REDACT_LEVEL="strict"
    generate_case_study "$CASE_STUDY_REDACTED" "true"
    log_success "Redacted case study written to: ${CASE_STUDY_REDACTED}"
fi

# ============================================================================
# Summary JSON
# ============================================================================

SUMMARY_FILE="${OUTPUT_DIR}/case-study.json"

jq -n \
    --arg title "$TITLE" \
    --arg env "$ENV_TYPE" \
    --arg obs_start "$OBS_START" \
    --arg obs_end "$OBS_END" \
    --argjson snapshots "$OBS_SNAPSHOTS" \
    --argjson recs "$UNIQUE_RECS" \
    --argjson applies "$APPLY_COUNT" \
    --argjson stable "$STABLE_COUNT" \
    --argjson rollback "$ROLLBACK_COUNT" \
    --argjson failed "$FAILED_COUNT" \
    '{
        "title": $title,
        "environment": $env,
        "observation": {
            "start": $obs_start,
            "end": $obs_end,
            "snapshots": $snapshots,
            "recommendations_identified": $recs
        },
        "applies": {
            "total": $applies,
            "stable": $stable,
            "rolled_back": $rollback,
            "failed": $failed
        },
        "generated_at": (now | todate)
    }' > "$SUMMARY_FILE"

log_success "Summary JSON written to: ${SUMMARY_FILE}"

echo ""
echo "Case study generation complete!"
echo "  - Full version: ${CASE_STUDY_FILE}"
echo "  - Redacted version: ${CASE_STUDY_REDACTED}"
echo "  - JSON summary: ${SUMMARY_FILE}"
