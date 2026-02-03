# Vaanyze Pilot Guide

This guide walks you through running a Vaanyze pilot from start to finish.

## Prerequisites

Before starting a pilot, ensure you have:

- [ ] **PostgreSQL 12+** with `pg_stat_statements` extension enabled
- [ ] **Network access** to the PostgreSQL database
- [ ] **Docker** installed (for running Vaanyze container)
- [ ] **bash, curl** available (standard on macOS/Linux)
- [ ] **jq** installed (recommended, but scripts work without it)

### Database Requirements

```sql
-- Verify pg_stat_statements is installed
SELECT * FROM pg_extension WHERE extname = 'pg_stat_statements';

-- Check query data is being collected
SELECT count(*) FROM pg_stat_statements;
-- Should return > 0 if queries have been running
```

### User Permissions

For **observe-only** pilots:
```sql
GRANT SELECT ON pg_stat_statements TO your_user;
```

For **apply** pilots (creating indexes):
```sql
GRANT CREATE ON SCHEMA public TO your_user;
-- Or for specific tables:
GRANT CREATE ON TABLE your_table TO your_user;
```

---

## Quick Start

### 1. Start Vaanyze

```bash
# Start Vaanyze server
docker run -d \
  --name vaanyze \
  -e DATABASE_URL="postgresql://user:pass@host:5432/dbname" \
  -e OPERATION_MODE=observe_only \
  -p 9090:9090 \
  ghcr.io/vaanyze/vaanyze:latest

# Or run locally
cargo run -- serve --port 9090 -d $DATABASE_URL
```

### 2. Run Doctor Check

```bash
cd scripts/pilot

# Check readiness
./run_pilot.sh doctor

# Or directly:
./pilot_doctor.sh
```

The doctor checks:
- Vaanyze connectivity
- pg_stat_statements availability
- CREATE/DROP INDEX permissions
- Kill switch status
- Existing Vaanyze indexes

### 3. Run 48-Hour Observation

```bash
# Full 48-hour observation (default)
./run_pilot.sh observe

# Shorter observation for testing
./run_pilot.sh observe --hours 4 --interval 5

# With explicit output directory
./run_pilot.sh observe --hours 48 --output-dir ./my-pilot
```

The observation will:
- Take snapshots every 15 minutes (configurable)
- Record slow queries, recommendations, and DB metrics
- Generate a timeline file and preliminary case study
- Resume automatically if interrupted

### 4. Apply a Recommendation (Optional)

After observing, you can apply a validated recommendation:

```bash
# Auto-pick the top validated recommendation
./run_pilot.sh apply-one --pick-top-validated

# Apply specific recommendation
./run_pilot.sh apply-one --recommendation-id <id>

# With longer verification window
./run_pilot.sh apply-one --pick-top-validated --verification-minutes 60
```

### 5. Generate Case Study

```bash
# Generate from most recent pilot artifacts
./run_pilot.sh case-study

# From specific pilot directory
./run_pilot.sh case-study --input-dir ./pilot-artifacts/20240115_120000

# With strict redaction (for sharing externally)
./run_pilot.sh case-study --input-dir ./pilot-artifacts/20240115_120000 --redact-level strict
```

---

## Detailed Workflows

### Observation-Only Pilot (48 hours)

This is the safest way to evaluate Vaanyze. No changes are made to your database.

1. **Start Vaanyze in observe_only mode**
   ```bash
   export OPERATION_MODE=observe_only
   cargo run -- serve --port 9090 -d $DATABASE_URL
   ```

2. **Run the pilot**
   ```bash
   ./run_pilot.sh observe --hours 48
   ```

3. **What happens:**
   - Doctor checks run automatically
   - Snapshots collected every 15 minutes
   - Timeline saved to `pilot-artifacts/<timestamp>/timeline.jsonl`
   - Case study generated at the end

4. **Review results:**
   - `pilot-artifacts/<timestamp>/observe_summary.json` - Summary statistics
   - `pilot-artifacts/<timestamp>/case-study.md` - Human-readable case study
   - `pilot-artifacts/<timestamp>/timeline.jsonl` - Raw timeline data

### Apply + Verify Pilot

This pilot applies a single recommendation and verifies it works.

1. **First, complete an observation pilot** (or at least run doctor)

2. **Validate a recommendation**
   ```bash
   curl -X POST http://localhost:9090/api/v1/index/validate \
     -H "Content-Type: application/json" \
     -d '{
       "query": "SELECT * FROM orders WHERE customer_id = $1",
       "table_name": "orders",
       "columns": ["customer_id"]
     }'
   ```

3. **Review the validation**
   - Check `validated: true`
   - Check `index_used: true`
   - Check `cost_improvement_pct` is meaningful (>10%)

4. **Apply the recommendation**
   ```bash
   ./run_pilot.sh apply-one --recommendation-id <id>
   ```

5. **What happens:**
   - Index created with `CREATE INDEX CONCURRENTLY`
   - Verification window starts (default 30 minutes)
   - idx_scan monitored for actual usage
   - Auto-rollback if no improvement detected

6. **Review results:**
   - `pilot-artifacts/<timestamp>/apply_<id>.json` - Full apply report
   - `pilot-artifacts/<timestamp>/case-study.md` - Updated case study

---

## How Safe Apply Works

### State Machine

Recommendations flow through these states:

```
NEW → VALIDATED → APPLIED → VERIFYING → STABLE
                              ↓
                         ROLLED_BACK
```

### Safety Guarantees

1. **EXPLAIN Proof Required**
   - Every recommendation is validated before apply
   - Candidate index created, EXPLAIN run, index dropped
   - Only if planner uses the index AND cost improves is it marked VALIDATED

2. **Non-Blocking Index Creation**
   - Uses `CREATE INDEX CONCURRENTLY`
   - No table locks during build
   - Safe for production workloads

3. **Verification Window**
   - Default: 30 minutes (configurable)
   - Monitors `idx_scan` counter
   - Monitors query latency if available
   - **Note**: The pilot scripts poll the status endpoint during verification. Background auto-verification is not yet implemented in the HTTP server - use the pilot scripts for verification.

4. **Automatic Rollback**
   - If idx_scan stays at 0: index not used, roll back
   - If latency regresses: roll back
   - Uses `DROP INDEX CONCURRENTLY`
   - **Note**: Rollback is triggered by pilot scripts polling, not background server process.

### Manual Rollback

You can always manually rollback:

```bash
curl -X POST http://localhost:9090/api/v1/index/rollback \
  -H "Content-Type: application/json" \
  -d '{
    "recommendation_id": "<id>",
    "reason": "Manual rollback requested",
    "triggered_by": "pilot_operator"
  }'
```

---

## Troubleshooting

### "Vaanyze not reachable"

```bash
# Check if Vaanyze is running
curl http://localhost:9090/api/v1/health

# Check Docker logs
docker logs vaanyze

# Common issues:
# - Wrong port (default is 9090)
# - DATABASE_URL not set or incorrect
# - Firewall blocking connection
```

### "pg_stat_statements not available"

```sql
-- Check if extension is installed
SELECT * FROM pg_extension WHERE extname = 'pg_stat_statements';

-- Install if missing (requires superuser or rds_superuser)
CREATE EXTENSION IF NOT EXISTS pg_stat_statements;

-- Check postgresql.conf includes:
-- shared_preload_libraries = 'pg_stat_statements'
-- (requires restart)
```

### "No CREATE INDEX permission"

Vaanyze will automatically run in observe-only mode. To enable apply:

```sql
-- Grant index creation permission
GRANT CREATE ON SCHEMA public TO your_user;

-- Or grant ownership
ALTER TABLE your_table OWNER TO your_user;
```

### "Kill switch is active"

```bash
# Disable kill switch
curl -X POST http://localhost:9090/api/v1/system/kill-switch \
  -H "Content-Type: application/json" \
  -d '{"enabled": false}'
```

### "Recommendation not validated"

You must validate a recommendation before applying it:

```bash
# Check current state
curl http://localhost:9090/api/v1/index/status/<recommendation_id>

# If state is NEW, run validation first
curl -X POST http://localhost:9090/api/v1/index/validate \
  -H "Content-Type: application/json" \
  -d '{...}'
```

### Observation script stops unexpectedly

The observation script is resumable. Just run it again with `--resume`:

```bash
./pilot_observe.sh --resume --output-dir ./pilot-artifacts/<timestamp>
```

---

## Artifacts Reference

After a complete pilot, you'll have these files:

```
pilot-artifacts/<timestamp>/
├── doctor.json              # Readiness check results
├── timeline.jsonl           # Raw observation data (JSON Lines)
├── latest.json              # Most recent snapshot
├── observe_summary.json     # Observation summary statistics
├── apply_<id>.json          # Apply workflow report (if applied)
├── apply_<id>_before.json   # Pre-apply state
├── case-study.md            # Human-readable case study
├── case-study.redacted.md   # Redacted version (for sharing)
└── case-study.json          # Case study summary
```

---

## Environment Variables

| Variable | Description | Default |
|----------|-------------|---------|
| `VAANYZE_BASE_URL` | Vaanyze HTTP endpoint | `http://localhost:9090` |
| `DATABASE_URL` | PostgreSQL connection string | (none) |
| `OPERATION_MODE` | Operation mode | `observe_only` |

---

## FAQ

**Q: Can I run the pilot on production?**
A: Yes, when using observe-only mode. For apply mode, we recommend staging first.

**Q: How much overhead does observation add?**
A: Minimal. Observation reads from pg_stat_statements and Vaanyze APIs, no database writes.

**Q: What if an applied index causes problems?**
A: The verification window will detect issues and auto-rollback. You can also manually rollback anytime.

**Q: Can I customize the observation interval?**
A: Yes, use `--interval N` where N is minutes (default: 15).

**Q: How do I share results without exposing table names?**
A: Use `--redact-level strict` when generating the case study.
