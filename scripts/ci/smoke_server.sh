#!/usr/bin/env bash
# CI script: Start server and verify basic health endpoints
# Includes retry loop for startup and cleanup on exit
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
    if [ -n "$SERVER_PID" ] && kill -0 "$SERVER_PID" 2>/dev/null; then
        echo "Stopping server (PID $SERVER_PID)..."
        kill "$SERVER_PID" 2>/dev/null || true
        wait "$SERVER_PID" 2>/dev/null || true
    fi
    echo "Cleanup complete"
}
trap cleanup EXIT

echo "=== Server Smoke Test ==="
echo "Port: $PORT"
echo "Database: ${DATABASE_URL:-<not set>}"

# Binary name (package name from Cargo.toml)
BINARY_NAME="pg-query-advisor"

# Build if needed
if [ ! -f "$REPO_ROOT/target/release/$BINARY_NAME" ]; then
    echo "Building release binary..."
    cargo build --release --manifest-path "$REPO_ROOT/Cargo.toml"
fi

# Start server in background
echo "Starting server on port $PORT..."
"$REPO_ROOT/target/release/$BINARY_NAME" serve --port "$PORT" &
SERVER_PID=$!
echo "Server PID: $SERVER_PID"

# Wait for server readiness with retry loop
echo "Waiting for server to be ready (max ${MAX_WAIT}s)..."
READY=0
for i in $(seq 1 $MAX_WAIT); do
    if curl -sf "$BASE_URL/api/v1/health" >/dev/null 2>&1; then
        READY=1
        echo "Server ready after ${i}s"
        break
    fi
    # Check if process died
    if ! kill -0 "$SERVER_PID" 2>/dev/null; then
        echo "❌ FAIL: Server process died unexpectedly"
        exit 1
    fi
    sleep 1
done

if [ "$READY" -eq 0 ]; then
    echo "❌ FAIL: Server did not become ready within ${MAX_WAIT}s"
    exit 1
fi

# Test endpoints
FAILED=0

echo ""
echo "=== Testing Endpoints ==="

# Test 1: /api/v1/health
echo -n "GET /api/v1/health ... "
RESPONSE=$(curl -sf -w "\n%{http_code}" "$BASE_URL/api/v1/health" 2>&1) || {
    echo "❌ FAIL"
    echo "Response: $RESPONSE"
    FAILED=1
}
HTTP_CODE=$(echo "$RESPONSE" | tail -1)
if [ "$HTTP_CODE" = "200" ]; then
    echo "✅ 200 OK"
else
    echo "❌ FAIL (HTTP $HTTP_CODE)"
    echo "$RESPONSE" | head -n -1
    FAILED=1
fi

# Test 2: /api/v1/system/environment
echo -n "GET /api/v1/system/environment ... "
RESPONSE=$(curl -sf -w "\n%{http_code}" "$BASE_URL/api/v1/system/environment" 2>&1) || {
    echo "❌ FAIL"
    echo "Response: $RESPONSE"
    FAILED=1
}
HTTP_CODE=$(echo "$RESPONSE" | tail -1)
if [ "$HTTP_CODE" = "200" ]; then
    echo "✅ 200 OK"
else
    echo "❌ FAIL (HTTP $HTTP_CODE)"
    echo "$RESPONSE" | head -n -1
    FAILED=1
fi

# Test 3: /api/v1/system/health
echo -n "GET /api/v1/system/health ... "
RESPONSE=$(curl -sf -w "\n%{http_code}" "$BASE_URL/api/v1/system/health" 2>&1) || {
    echo "❌ FAIL"
    echo "Response: $RESPONSE"
    FAILED=1
}
HTTP_CODE=$(echo "$RESPONSE" | tail -1)
if [ "$HTTP_CODE" = "200" ]; then
    echo "✅ 200 OK"
else
    echo "❌ FAIL (HTTP $HTTP_CODE)"
    echo "$RESPONSE" | head -n -1
    FAILED=1
fi

echo ""
if [ "$FAILED" -eq 0 ]; then
    echo "✅ All smoke tests passed"
    exit 0
else
    echo "❌ Some smoke tests failed"
    exit 1
fi
