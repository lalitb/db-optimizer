#!/bin/bash
#
# Vaanyze Pilot Observe
#
# Runs a read-only observation session, collecting metrics over time.
#
# Usage:
#   ./pilot_observe.sh [OPTIONS]
#
# Options:
#   --hours N            Duration to observe (default: 48)
#   --interval N         Snapshot interval in minutes (default: 15)
#   --output-dir DIR     Directory for artifacts
#   --resume             Resume existing observation (append to timeline)
#
# Environment:
#   DATABASE_URL         PostgreSQL connection string (for direct queries)
#   VAANYZE_BASE_URL     Vaanyze HTTP endpoint (default: http://localhost:9090)
#
set -euo pipefail

# Colors
if [[ -t 1 ]]; then
    GREEN='\033[0;32m'
    YELLOW='\033[1;33m'
    BLUE='\033[0;34m'
    NC='\033[0m'
else
    GREEN='' YELLOW='' BLUE='' NC=''
fi

# Defaults
VAANYZE_BASE_URL="${VAANYZE_BASE_URL:-http://localhost:9090}"
DATABASE_URL="${DATABASE_URL:-}"
HOURS=48
INTERVAL_MINUTES=15
OUTPUT_DIR=""
RESUME=false
TIMESTAMP=$(date +%Y%m%d_%H%M%S)

# Parse arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        --hours) HOURS="$2"; shift 2 ;;
        --interval) INTERVAL_MINUTES="$2"; shift 2 ;;
        --output-dir) OUTPUT_DIR="$2"; shift 2 ;;
        --resume) RESUME=true; shift ;;
        -h|--help)
            echo "Usage: $0 [--hours N] [--interval N] [--output-dir DIR] [--resume]"
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

TIMELINE_FILE="${OUTPUT_DIR}/timeline.jsonl"
LATEST_FILE="${OUTPUT_DIR}/latest.json"

# Check for jq
HAS_JQ=true
if ! command -v jq &> /dev/null; then
    HAS_JQ=false
    echo "Warning: jq not found - output will be limited"
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

# JSON helper for when jq is not available
json_extract() {
    local json="$1"
    local key="$2"
    if [[ "$HAS_JQ" == "true" ]]; then
        echo "$json" | jq -r ".$key // empty" 2>/dev/null || echo ""
    else
        echo "$json" | grep -o "\"$key\":[^,}]*" | sed 's/.*://' | tr -d ' "' || echo ""
    fi
}

# ============================================================================
# Snapshot Collection
# ============================================================================

collect_health_snapshot() {
    local health=""
    if health=$(curl -sf "${VAANYZE_BASE_URL}/api/v1/health" 2>/dev/null); then
        if [[ "$HAS_JQ" == "true" ]]; then
            echo "$health" | jq -r '.data // .'
        else
            echo "$health"
        fi
    else
        echo '{"status": "unavailable"}'
    fi
}

collect_slow_queries() {
    # Try Vaanyze endpoint first
    local queries=""
    if queries=$(curl -sf "${VAANYZE_BASE_URL}/api/v1/queries/slow?limit=10" 2>/dev/null); then
        if [[ "$HAS_JQ" == "true" ]]; then
            echo "$queries" | jq -r '.data // .'
        else
            echo "$queries"
        fi
        return
    fi
    
    # Fallback to direct psql if DATABASE_URL is available
    if [[ -n "$DATABASE_URL" ]]; then
        local result=""
        local sql="SELECT queryid, LEFT(query, 200) as query_preview, calls, total_exec_time / 1000 as total_secs, mean_exec_time as mean_ms, rows FROM pg_stat_statements WHERE queryid IS NOT NULL ORDER BY mean_exec_time DESC LIMIT 10;"
        result=$(psql "$DATABASE_URL" -t -A -F'|' -c "$sql" 2>/dev/null || echo "")
        if [[ -n "$result" ]]; then
            # Convert to JSON
            echo '{"queries": ['
            local first=true
            while IFS='|' read -r qid query calls total mean rows; do
                [[ -z "$qid" ]] && continue
                [[ "$first" != "true" ]] && echo ','
                first=false
                echo "{\"queryid\": \"$qid\", \"query\": \"$(echo "$query" | sed 's/"/\\"/g')\", \"calls\": $calls, \"total_secs\": $total, \"mean_ms\": $mean, \"rows\": $rows}"
            done <<< "$result"
            echo ']}'
        else
            echo '{"queries": [], "source": "psql", "error": "no data"}'
        fi
    else
        echo '{"queries": [], "error": "no data source"}'
    fi
}

collect_recommendations() {
    local recs=""
    if recs=$(curl -sf "${VAANYZE_BASE_URL}/api/v1/recommendations?limit=20" 2>/dev/null); then
        if [[ "$HAS_JQ" == "true" ]]; then
            echo "$recs" | jq -r '.data // .'
        else
            echo "$recs"
        fi
    else
        echo '{"recommendations": []}'
    fi
}

collect_validations() {
    # Get all validation statuses from audit log
    local audit=""
    if audit=$(curl -sf "${VAANYZE_BASE_URL}/api/v1/index/audit" 2>/dev/null); then
        if [[ "$HAS_JQ" == "true" ]]; then
            echo "$audit" | jq -r '.data // .'
        else
            echo "$audit"
        fi
    else
        echo '{"events": []}'
    fi
}

collect_db_metrics() {
    if [[ -z "$DATABASE_URL" ]]; then
        echo '{"error": "no DATABASE_URL"}'
        return
    fi
    
    # Collect basic DB health metrics
    local metrics=""
    local sql="SELECT (SELECT count(*) FROM pg_stat_activity) as connections, (SELECT count(*) FROM pg_stat_activity WHERE state = 'active') as active_queries, (SELECT sum(blks_hit)::float / NULLIF(sum(blks_hit + blks_read), 0) * 100 FROM pg_stat_database) as cache_hit_ratio, (SELECT checkpoints_timed + checkpoints_req FROM pg_stat_bgwriter) as total_checkpoints, (SELECT buffers_checkpoint FROM pg_stat_bgwriter) as buffers_checkpoint;"
    metrics=$(psql "$DATABASE_URL" -t -A -F'|' -c "$sql" 2>/dev/null || echo "")
    
    if [[ -n "$metrics" ]]; then
        IFS='|' read -r conns active cache_hit checkpoints buffers <<< "$metrics"
        echo "{\"connections\": ${conns:-0}, \"active_queries\": ${active:-0}, \"cache_hit_ratio\": ${cache_hit:-0}, \"total_checkpoints\": ${checkpoints:-0}, \"buffers_checkpoint\": ${buffers:-0}}"
    else
        echo '{"error": "could not collect metrics"}'
    fi
}

collect_managed_indexes() {
    local indexes=""
    if indexes=$(curl -sf "${VAANYZE_BASE_URL}/api/v1/system/indexes" 2>/dev/null); then
        if [[ "$HAS_JQ" == "true" ]]; then
            echo "$indexes" | jq -r '.data // .'
        else
            echo "$indexes"
        fi
    else
        echo '{"indexes": [], "total": 0}'
    fi
}

# ============================================================================
# Main Observation Loop
# ============================================================================

log_info "Starting observation session"
log_info "  Duration: ${HOURS} hours"
log_info "  Interval: ${INTERVAL_MINUTES} minutes"
log_info "  Output: ${OUTPUT_DIR}"

if [[ "$RESUME" == "true" ]] && [[ -f "$TIMELINE_FILE" ]]; then
    EXISTING_SNAPSHOTS=$(wc -l < "$TIMELINE_FILE")
    log_info "  Resuming: ${EXISTING_SNAPSHOTS} existing snapshots"
fi

TOTAL_SNAPSHOTS=$((HOURS * 60 / INTERVAL_MINUTES))
INTERVAL_SECONDS=$((INTERVAL_MINUTES * 60))
SNAPSHOT_COUNT=0
START_TIME=$(date +%s)
END_TIME=$((START_TIME + HOURS * 3600))

log_info "  Expected snapshots: ${TOTAL_SNAPSHOTS}"
log_info ""

while [[ $(date +%s) -lt $END_TIME ]]; do
    SNAPSHOT_COUNT=$((SNAPSHOT_COUNT + 1))
    CURRENT_TIME=$(date -Iseconds)
    
    log_info "Collecting snapshot ${SNAPSHOT_COUNT}/${TOTAL_SNAPSHOTS}..."
    
    # Collect all data
    HEALTH=$(collect_health_snapshot)
    SLOW_QUERIES=$(collect_slow_queries)
    RECOMMENDATIONS=$(collect_recommendations)
    VALIDATIONS=$(collect_validations)
    DB_METRICS=$(collect_db_metrics)
    INDEXES=$(collect_managed_indexes)
    
    # Build snapshot
    if [[ "$HAS_JQ" == "true" ]]; then
        SNAPSHOT=$(jq -n \
            --arg ts "$CURRENT_TIME" \
            --argjson health "$HEALTH" \
            --argjson queries "$SLOW_QUERIES" \
            --argjson recs "$RECOMMENDATIONS" \
            --argjson vals "$VALIDATIONS" \
            --argjson db "$DB_METRICS" \
            --argjson idx "$INDEXES" \
            '{
                "timestamp": $ts,
                "snapshot_number": '"$SNAPSHOT_COUNT"',
                "health": $health,
                "slow_queries": $queries,
                "recommendations": $recs,
                "validations": $vals,
                "db_metrics": $db,
                "managed_indexes": $idx
            }')
        
        # Write to timeline (JSONL format)
        echo "$SNAPSHOT" | jq -c . >> "$TIMELINE_FILE"
        
        # Write latest snapshot for easy access
        echo "$SNAPSHOT" | jq . > "$LATEST_FILE"
    else
        # Basic JSON without jq
        cat >> "$TIMELINE_FILE" << EOF
{"timestamp": "$CURRENT_TIME", "snapshot_number": $SNAPSHOT_COUNT, "health": $HEALTH, "slow_queries": $SLOW_QUERIES}
EOF
    fi
    
    # Print summary
    if [[ "$HAS_JQ" == "true" ]]; then
        QUERY_COUNT=$(echo "$SLOW_QUERIES" | jq -r '.queries | length // 0' 2>/dev/null || echo "0")
        REC_COUNT=$(echo "$RECOMMENDATIONS" | jq -r '.recommendations | length // 0' 2>/dev/null || echo "0")
        log_success "  Slow queries: ${QUERY_COUNT}, Recommendations: ${REC_COUNT}"
    else
        log_success "  Snapshot recorded"
    fi
    
    # Calculate time remaining
    ELAPSED=$(($(date +%s) - START_TIME))
    REMAINING=$((END_TIME - $(date +%s)))
    HOURS_LEFT=$((REMAINING / 3600))
    MINS_LEFT=$(((REMAINING % 3600) / 60))
    
    log_info "  Time remaining: ${HOURS_LEFT}h ${MINS_LEFT}m"
    log ""
    
    # Sleep until next interval (unless we're done)
    if [[ $(date +%s) -lt $END_TIME ]]; then
        sleep "$INTERVAL_SECONDS" || break
    fi
done

# ============================================================================
# Generate Summary
# ============================================================================

log_info "Observation complete!"
log_info ""

SUMMARY_FILE="${OUTPUT_DIR}/observe_summary.json"

if [[ "$HAS_JQ" == "true" ]] && [[ -f "$TIMELINE_FILE" ]]; then
    # Generate summary statistics
    TOTAL_SNAPSHOTS_ACTUAL=$(wc -l < "$TIMELINE_FILE")
    
    # Find top slow queries across all snapshots
    TOP_QUERIES=$(cat "$TIMELINE_FILE" | jq -s '
        [.[].slow_queries.queries[]? | select(.mean_ms != null)] 
        | group_by(.queryid) 
        | map({queryid: .[0].queryid, query: .[0].query, max_mean_ms: (map(.mean_ms) | max), observations: length})
        | sort_by(-.max_mean_ms)
        | .[0:5]
    ' 2>/dev/null || echo '[]')
    
    # Count recommendations seen
    REC_COUNT=$(cat "$TIMELINE_FILE" | jq -s '[.[].recommendations.recommendations[]? | .id] | unique | length' 2>/dev/null || echo "0")
    
    jq -n \
        --arg start "$(head -1 "$TIMELINE_FILE" | jq -r '.timestamp')" \
        --arg end "$(tail -1 "$TIMELINE_FILE" | jq -r '.timestamp')" \
        --argjson snapshots "$TOTAL_SNAPSHOTS_ACTUAL" \
        --argjson recs "$REC_COUNT" \
        --argjson top_queries "$TOP_QUERIES" \
        '{
            "observation_period": {
                "start": $start,
                "end": $end,
                "snapshots_collected": $snapshots
            },
            "unique_recommendations": $recs,
            "top_slow_queries": $top_queries
        }' > "$SUMMARY_FILE"
    
    log_info "Summary written to: ${SUMMARY_FILE}"
fi

log_info "Timeline written to: ${TIMELINE_FILE}"
log_info ""
log_success "Observation session complete"

exit 0
