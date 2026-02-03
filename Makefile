# Vaanyze Makefile
#
# Build, test, and benchmark targets
#

.PHONY: all build test clean bench-up bench-down bench-all bench-summary bench-summary-ci \
        scenario-missing-index scenario-composite scenario-join-path \
        scenario-order-by-limit scenario-write-heavy scenario-wal-pressure \
        pilot-doctor pilot-observe pilot-apply pilot-case-study

# Default target
all: build

# ═══════════════════════════════════════════════════════════════════
# Build Targets
# ═══════════════════════════════════════════════════════════════════

build:
	cargo build --release

build-debug:
	cargo build

test:
	cargo test

test-verbose:
	cargo test -- --nocapture

clippy:
	cargo clippy -- -D warnings

fmt:
	cargo fmt

fmt-check:
	cargo fmt -- --check

audit:
	cargo audit

# ═══════════════════════════════════════════════════════════════════
# Benchmark Pack Targets
# ═══════════════════════════════════════════════════════════════════

BENCH_DIR := examples/benchmark-pack
SCRIPTS_DIR := scripts/bench

# Start benchmark PostgreSQL
bench-up:
	@echo "Starting benchmark PostgreSQL..."
	cd $(BENCH_DIR) && docker-compose up -d postgres
	@echo "Waiting for PostgreSQL to be healthy..."
	@sleep 5
	@cd $(BENCH_DIR) && docker-compose exec postgres pg_isready -U postgres -d benchdb || \
		(echo "Waiting more..." && sleep 5)
	@echo "PostgreSQL is ready!"

# Stop and clean benchmark environment
bench-down:
	@echo "Stopping benchmark environment..."
	cd $(BENCH_DIR) && docker-compose down -v

# Run all benchmark scenarios
bench-all: bench-up
	@echo "Running all benchmark scenarios..."
	$(SCRIPTS_DIR)/run_all.sh

# Start Vaanyze alongside PostgreSQL
bench-with-vaanyze: build
	@echo "Starting benchmark environment with Vaanyze..."
	cd $(BENCH_DIR) && docker-compose --profile with-vaanyze up -d

# Individual scenario targets
scenario-missing-index: bench-up
	$(SCRIPTS_DIR)/run_scenario.sh 01-missing-index-where

scenario-composite: bench-up
	$(SCRIPTS_DIR)/run_scenario.sh 02-composite-index

scenario-join-path: bench-up
	$(SCRIPTS_DIR)/run_scenario.sh 03-join-path

scenario-order-by-limit: bench-up
	$(SCRIPTS_DIR)/run_scenario.sh 04-order-by-limit

scenario-write-heavy: bench-up
	$(SCRIPTS_DIR)/run_scenario.sh 05-write-heavy-rollback

scenario-wal-pressure: bench-up
	$(SCRIPTS_DIR)/run_scenario.sh 06-wal-pressure-negative

# View benchmark results
bench-results:
	@echo "Benchmark Results:"
	@echo "=================="
	@cat $(BENCH_DIR)/results/summary.json 2>/dev/null | python3 -m json.tool || \
		echo "No results found. Run 'make bench-all' first."

# Generate benchmark summary (markdown + JSON)
bench-summary:
	$(SCRIPTS_DIR)/summarize_results.sh

# Generate summary for CI (exit non-zero on failures)
bench-summary-ci:
	$(SCRIPTS_DIR)/summarize_results.sh --ci

# Clean benchmark results
bench-clean:
	rm -rf $(BENCH_DIR)/results/*.json

# ═══════════════════════════════════════════════════════════════════
# Docker Targets
# ═══════════════════════════════════════════════════════════════════

docker-build:
	docker build -t vaanyze:latest .

docker-run: docker-build
	docker run --rm -it \
		-e DATABASE_URL=postgresql://postgres:postgres@host.docker.internal:5433/benchdb \
		vaanyze:latest analyze

# ═══════════════════════════════════════════════════════════════════
# Development Targets
# ═══════════════════════════════════════════════════════════════════

# Run the development test database
dev-db:
	docker-compose up -d postgres

dev-db-down:
	docker-compose down

# Run local analysis
analyze:
	cargo run --release -- analyze -d postgresql://postgres:postgres@localhost:5433/benchdb

# Run HTTP server
serve:
	cargo run --release -- serve --port 9090 -d postgresql://postgres:postgres@localhost:5433/benchdb

# ═══════════════════════════════════════════════════════════════════
# Pilot Targets
# ═══════════════════════════════════════════════════════════════════

PILOT_DIR := scripts/pilot

# Run pilot doctor checks
pilot-doctor:
	$(PILOT_DIR)/run_pilot.sh doctor

# Start pilot observation (default 48 hours)
pilot-observe:
	$(PILOT_DIR)/run_pilot.sh observe --hours 48

# Start short pilot observation (4 hours, for testing)
pilot-observe-short:
	$(PILOT_DIR)/run_pilot.sh observe --hours 4 --interval 5

# Apply top validated recommendation
pilot-apply:
	$(PILOT_DIR)/run_pilot.sh apply-one --pick-top-validated

# Generate case study from most recent pilot
pilot-case-study:
	$(PILOT_DIR)/run_pilot.sh case-study

# ═══════════════════════════════════════════════════════════════════
# Documentation
# ═══════════════════════════════════════════════════════════════════

docs:
	@echo "Documentation available at:"
	@echo "  - README.md"
	@echo "  - docs/benchmark-pack.md"
	@echo "  - docs/pilot-guide.md"
	@echo "  - docs/design-partner-checklist.md"
	@echo "  - docs/QUICKSTART.md"
	@echo "  - docs/SAFETY.md"
	@echo "  - docs/LIMITATIONS.md"
	@echo "  - docs/POSITIONING.md"

# ═══════════════════════════════════════════════════════════════════
# Help
# ═══════════════════════════════════════════════════════════════════

help:
	@echo "Vaanyze Makefile"
	@echo ""
	@echo "Build targets:"
	@echo "  make build          - Build release binary"
	@echo "  make build-debug    - Build debug binary"
	@echo "  make test           - Run all tests"
	@echo "  make clippy         - Run linter"
	@echo "  make fmt            - Format code"
	@echo ""
	@echo "Benchmark targets:"
	@echo "  make bench-up       - Start benchmark PostgreSQL"
	@echo "  make bench-down     - Stop benchmark environment"
	@echo "  make bench-all      - Run all benchmark scenarios"
	@echo "  make bench-results  - View benchmark results"
	@echo "  make bench-summary  - Generate summary report"
	@echo "  make bench-summary-ci - Generate summary for CI (fail on errors)"
	@echo ""
	@echo "Individual scenarios:"
	@echo "  make scenario-missing-index    - Missing index on WHERE"
	@echo "  make scenario-composite        - Composite index"
	@echo "  make scenario-join-path        - Join FK index"
	@echo "  make scenario-order-by-limit   - ORDER BY + LIMIT"
	@echo "  make scenario-write-heavy      - Write-heavy rollback test"
	@echo "  make scenario-wal-pressure     - WAL pressure negative test"
	@echo ""
	@echo "Pilot targets:"
	@echo "  make pilot-doctor       - Run pilot readiness checks"
	@echo "  make pilot-observe      - Run 48-hour observation"
	@echo "  make pilot-observe-short - Run 4-hour observation (testing)"
	@echo "  make pilot-apply        - Apply top validated recommendation"
	@echo "  make pilot-case-study   - Generate case study"
	@echo ""
	@echo "Development targets:"
	@echo "  make dev-db         - Start development database"
	@echo "  make analyze        - Run analysis on benchmark DB"
	@echo "  make serve          - Start HTTP server"
