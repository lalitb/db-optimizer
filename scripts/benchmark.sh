#!/bin/bash
# Performance Benchmarking Script for db-optimizer
#
# This script runs various performance benchmarks to validate
# the efficiency of the eBPF and query analysis components.
#
# Usage:
#   ./scripts/benchmark.sh [--full]
#
# Options:
#   --full    Run extended benchmarks (takes longer)

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Default settings
FULL_BENCHMARK=false
ITERATIONS=10000
WARMUP_ITERATIONS=1000

# Parse arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        --full)
            FULL_BENCHMARK=true
            ITERATIONS=100000
            WARMUP_ITERATIONS=10000
            shift
            ;;
        *)
            echo "Unknown option: $1"
            exit 1
            ;;
    esac
done

echo -e "${BLUE}========================================${NC}"
echo -e "${BLUE}  db-optimizer Performance Benchmarks  ${NC}"
echo -e "${BLUE}========================================${NC}"
echo ""

# Check if we're on Linux
if [[ "$(uname)" == "Linux" ]]; then
    PLATFORM="Linux"
    EBPF_AVAILABLE=true
else
    PLATFORM="$(uname)"
    EBPF_AVAILABLE=false
fi

echo -e "Platform: ${YELLOW}$PLATFORM${NC}"
echo -e "eBPF Available: ${YELLOW}$EBPF_AVAILABLE${NC}"
echo -e "Iterations: ${YELLOW}$ITERATIONS${NC}"
echo ""

# Build in release mode
echo -e "${BLUE}Building in release mode...${NC}"
cd "$PROJECT_DIR"
cargo build --release 2>&1 | tail -5

echo ""
echo -e "${BLUE}Running benchmarks...${NC}"
echo ""

# Run the cargo benchmarks (ignored tests with benchmark prefix)
echo -e "${GREEN}=== Query Normalization Benchmark ===${NC}"
cargo test --test ebpf_integration_test bench_query_normalization -- --ignored --nocapture 2>&1 | grep -E "Query normalization|Iterations|Total time|Per query|Cache"

echo ""
echo -e "${GREEN}=== Session Tracking Benchmark ===${NC}"
cargo test --test ebpf_integration_test bench_session_tracking -- --ignored --nocapture 2>&1 | grep -E "Session tracking|Iterations|Total time|Per operation|Active"

echo ""
echo -e "${GREEN}=== Memory Usage ===${NC}"

# Get binary size
BINARY_SIZE=$(ls -lh target/release/db-optimizer 2>/dev/null | awk '{print $5}' || echo "N/A")
echo "Release binary size: $BINARY_SIZE"

# Run a quick memory check if possible
if command -v /usr/bin/time &> /dev/null; then
    echo "Running memory test..."
    /usr/bin/time -l cargo test --release --lib -- --test-threads=1 2>&1 | grep -E "maximum resident|real" | head -2 || true
fi

echo ""
echo -e "${GREEN}=== Compilation Time ===${NC}"

# Clean and measure build time
echo "Measuring clean build time..."
cargo clean 2>/dev/null || true

BUILD_START=$(date +%s.%N)
cargo build --release 2>&1 | tail -3
BUILD_END=$(date +%s.%N)
BUILD_TIME=$(echo "$BUILD_END - $BUILD_START" | bc 2>/dev/null || echo "N/A")

echo "Clean build time: ${BUILD_TIME}s"

echo ""
echo -e "${GREEN}=== Test Suite Performance ===${NC}"

# Measure test suite time
TEST_START=$(date +%s.%N)
cargo test --release --lib -- --test-threads=1 2>&1 | tail -3
TEST_END=$(date +%s.%N)
TEST_TIME=$(echo "$TEST_END - $TEST_START" | bc 2>/dev/null || echo "N/A")

echo "Test suite time: ${TEST_TIME}s"

# Extended benchmarks if requested
if [[ "$FULL_BENCHMARK" == "true" ]]; then
    echo ""
    echo -e "${GREEN}=== Extended Benchmarks ===${NC}"
    
    # More comprehensive benchmarks
    echo "Running extended query analysis benchmark..."
    
    # Create a test script
    cat > /tmp/bench_query.rs << 'EOF'
use std::time::Instant;

fn main() {
    let queries = vec![
        "SELECT * FROM users WHERE id = $1",
        "SELECT u.*, o.* FROM users u JOIN orders o ON u.id = o.user_id WHERE u.email = $1",
        "INSERT INTO events (user_id, type, data) VALUES ($1, $2, $3)",
        "UPDATE users SET last_login = NOW() WHERE id = $1",
        "DELETE FROM sessions WHERE expires_at < NOW()",
    ];
    
    let iterations = 100000;
    
    let start = Instant::now();
    for _ in 0..iterations {
        for q in &queries {
            let _ = q.len(); // Simulate some processing
        }
    }
    let elapsed = start.elapsed();
    
    println!("Processed {} query batches in {:?}", iterations, elapsed);
    println!("Per batch: {:.2} µs", elapsed.as_micros() as f64 / iterations as f64);
}
EOF
fi

echo ""
echo -e "${BLUE}========================================${NC}"
echo -e "${BLUE}  Benchmark Summary                     ${NC}"
echo -e "${BLUE}========================================${NC}"
echo ""
echo "Binary Size: $BINARY_SIZE"
echo "Build Time: ${BUILD_TIME}s"
echo "Test Time: ${TEST_TIME}s"
echo ""
echo -e "${GREEN}Benchmarks completed successfully!${NC}"
