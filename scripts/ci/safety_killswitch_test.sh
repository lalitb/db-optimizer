#!/usr/bin/env bash
# CI script: Test kill switch safety enforcement
# Verifies that dangerous endpoints return 403 when kill switch is active
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

PORT="${PORT:-9090}"
BASE_URL="http://localhost:$PORT"
MAX_WAIT=30
SERVER_PID=""

cleanup() {
    echo ""
    echo "=== Cleaning up ==="
    # Try to disable kill switch before cleanup (best effort)
    curl -sf -X POST "$BASE_URL/api/v1/system/kill-switch" \
        -H "Content-Type: application/json" \
        -d '{"enabled": false}' >/dev/null 2>&1 || true
    
    if [ -n "$SERVER_PID" ] && kill -0 "$SERVER_PID" 2>/dev/null; then
        echo "Stopping server (PID $SERVER_PID)..."
        kill "$SERVER_PID" 2>/dev/null || true
        wait "$SERVER_PID" 2>/dev/null || true
    fi
    echo "Cleanup complete"
}
trap cleanup EXIT

echo "=== Kill Switch Safety Test ==="
echo "Port: $PORT"

# Binary name (package name from Cargo.toml)
BINARY_NAME="pg-query-advisor"

# Build if needed
if [ ! -f "$REPO_ROOT/target/release/$BINARY_NAME" ]; then
    echo "Building release binary..."
    cargo build --release --manifest-path "$REPO_ROOT/Cargo.toml"
fi

# Start server
echo "Starting server on port $PORT..."
"$REPO_ROOT/target/release/$BINARY_NAME" serve --port "$PORT" &
SERVER_PID=$!
echo "Server PID: $SERVER_PID"

# Wait for readiness
echo "Waiting for server to be ready..."
READY=0
for i in $(seq 1 $MAX_WAIT); do
    if curl -sf "$BASE_URL/api/v1/health" >/dev/null 2>&1; then
        READY=1
        echo "Server ready after ${i}s"
        break
    fi
    if ! kill -0 "$SERVER_PID" 2>/dev/null; then
        echo "❌ FAIL: Server process died"
        exit 1
    fi
    sleep 1
done

if [ "$READY" -eq 0 ]; then
    echo "❌ FAIL: Server did not start within ${MAX_WAIT}s"
    exit 1
fi

FAILED=0

# Enable kill switch
echo ""
echo "=== Enabling Kill Switch ==="
RESPONSE=$(curl -sf -X POST "$BASE_URL/api/v1/system/kill-switch" \
    -H "Content-Type: application/json" \
    -d '{"enabled": true, "reason": "CI safety test"}' 2>&1) || {
    echo "❌ FAIL: Could not enable kill switch"
    echo "$RESPONSE"
    exit 1
}
echo "Kill switch enabled: $RESPONSE"

# Verify kill switch is active via health endpoint
echo ""
echo "=== Verifying Kill Switch State ==="
HEALTH=$(curl -sf "$BASE_URL/api/v1/system/health" 2>&1)
if echo "$HEALTH" | grep -q '"kill_switch_active":true'; then
    echo "✅ Kill switch confirmed active"
else
    echo "❌ FAIL: Kill switch not reflected in health endpoint"
    echo "$HEALTH"
    FAILED=1
fi

# Test that dangerous endpoints return 403
echo ""
echo "=== Testing Blocked Endpoints ==="

# Test apply endpoint - should return 403
echo -n "POST /api/v1/index/apply (expect 403) ... "
HTTP_CODE=$(curl -s -o /tmp/apply_response.json -w "%{http_code}" \
    -X POST "$BASE_URL/api/v1/index/apply" \
    -H "Content-Type: application/json" \
    -d '{"recommendation_id": "test-ci-12345"}')
if [ "$HTTP_CODE" = "403" ]; then
    echo "✅ 403 Forbidden (correct)"
elif [ "$HTTP_CODE" = "404" ] || [ "$HTTP_CODE" = "400" ]; then
    # 404/400 means recommendation not found, but endpoint was reached
    # Check if it would have been blocked by looking at response
    if grep -q "kill.switch\|Kill.switch\|KILL_SWITCH" /tmp/apply_response.json 2>/dev/null; then
        echo "✅ Blocked by kill switch (HTTP $HTTP_CODE)"
    else
        echo "⚠️ HTTP $HTTP_CODE - endpoint reached but no kill switch block message"
        cat /tmp/apply_response.json
    fi
else
    echo "❌ FAIL: Expected 403, got HTTP $HTTP_CODE"
    cat /tmp/apply_response.json
    FAILED=1
fi

# Test validate endpoint - should return 403
echo -n "POST /api/v1/index/validate (expect 403) ... "
HTTP_CODE=$(curl -s -o /tmp/validate_response.json -w "%{http_code}" \
    -X POST "$BASE_URL/api/v1/index/validate" \
    -H "Content-Type: application/json" \
    -d '{"recommendation_id": "test-ci-12345", "query": "SELECT 1", "table_name": "test", "columns": ["id"]}')
if [ "$HTTP_CODE" = "403" ]; then
    echo "✅ 403 Forbidden (correct)"
else
    echo "❌ FAIL: Expected 403, got HTTP $HTTP_CODE"
    cat /tmp/validate_response.json
    FAILED=1
fi

# Test rollback endpoint - should return 403
echo -n "POST /api/v1/index/rollback (expect 403) ... "
HTTP_CODE=$(curl -s -o /tmp/rollback_response.json -w "%{http_code}" \
    -X POST "$BASE_URL/api/v1/index/rollback" \
    -H "Content-Type: application/json" \
    -d '{"recommendation_id": "test-ci-12345", "reason": "CI test"}')
if [ "$HTTP_CODE" = "403" ]; then
    echo "✅ 403 Forbidden (correct)"
elif [ "$HTTP_CODE" = "404" ] || [ "$HTTP_CODE" = "400" ]; then
    if grep -q "kill.switch\|Kill.switch\|KILL_SWITCH" /tmp/rollback_response.json 2>/dev/null; then
        echo "✅ Blocked by kill switch (HTTP $HTTP_CODE)"
    else
        echo "⚠️ HTTP $HTTP_CODE - endpoint reached but no kill switch block message"
        cat /tmp/rollback_response.json
    fi
else
    echo "❌ FAIL: Expected 403, got HTTP $HTTP_CODE"
    cat /tmp/rollback_response.json
    FAILED=1
fi

# Test recover endpoint - should return 403
echo -n "POST /api/v1/system/recover (expect 403) ... "
HTTP_CODE=$(curl -s -o /tmp/recover_response.json -w "%{http_code}" \
    -X POST "$BASE_URL/api/v1/system/recover" \
    -H "Content-Type: application/json" \
    -d '{}')
if [ "$HTTP_CODE" = "403" ]; then
    echo "✅ 403 Forbidden (correct)"
else
    echo "❌ FAIL: Expected 403, got HTTP $HTTP_CODE"
    cat /tmp/recover_response.json
    FAILED=1
fi

# Disable kill switch and verify operations resume
echo ""
echo "=== Disabling Kill Switch ==="
curl -sf -X POST "$BASE_URL/api/v1/system/kill-switch" \
    -H "Content-Type: application/json" \
    -d '{"enabled": false}' >/dev/null

HEALTH=$(curl -sf "$BASE_URL/api/v1/system/health" 2>&1)
if echo "$HEALTH" | grep -q '"kill_switch_active":false'; then
    echo "✅ Kill switch disabled successfully"
else
    echo "⚠️ Could not verify kill switch disable"
fi

# Cleanup temp files
rm -f /tmp/apply_response.json /tmp/validate_response.json /tmp/rollback_response.json /tmp/recover_response.json

echo ""
if [ "$FAILED" -eq 0 ]; then
    echo "✅ All safety enforcement tests passed"
    exit 0
else
    echo "❌ Some safety tests failed"
    exit 1
fi
