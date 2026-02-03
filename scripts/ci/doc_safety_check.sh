#!/usr/bin/env bash
# CI script: Check docs for known false safety claims
# Fails if SAFETY.md contains phrases that are historically incorrect
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
SAFETY_MD="$REPO_ROOT/docs/SAFETY.md"

echo "=== Doc Safety Check ==="
echo "Checking: $SAFETY_MD"

FAILED=0

# Known false claims that must not appear
DISALLOWED_PHRASES=(
    "Never creates or drops indexes"
    "impossible to execute"
    "guaranteed to never"
    "zero risk"
    "100% safe"
)

for phrase in "${DISALLOWED_PHRASES[@]}"; do
    if grep -qF "$phrase" "$SAFETY_MD" 2>/dev/null; then
        echo "❌ FAIL: Found disallowed phrase: \"$phrase\""
        grep -n "$phrase" "$SAFETY_MD" || true
        FAILED=1
    fi
done

if [ "$FAILED" -eq 0 ]; then
    echo "✅ PASS: No disallowed safety claims found"
    exit 0
else
    echo ""
    echo "Doc safety check failed. Update docs to remove false claims."
    exit 1
fi
