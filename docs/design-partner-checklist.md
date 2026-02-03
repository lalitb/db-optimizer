# Vaanyze Design Partner Checklist

This document provides a step-by-step guide for design partners to evaluate Vaanyze in their environment. Follow these steps to safely validate Vaanyze's index recommendation capabilities.

## Prerequisites

### Database Requirements
- [ ] PostgreSQL 12+ (recommended: 14+)
- [ ] `pg_stat_statements` extension installed and enabled
- [ ] At least 1 hour of query traffic captured in pg_stat_statements
- [ ] Database user with:
  - `SELECT` on `pg_stat_statements`
  - `SELECT` on system catalogs (`pg_stat_user_indexes`, `pg_indexes`, etc.)
  - For apply mode: `CREATE INDEX` and `DROP INDEX` permissions

### Environment Requirements
- [ ] Network connectivity to PostgreSQL (direct or via SSH tunnel)
- [ ] Docker available (for running Vaanyze container)
- [ ] 512MB RAM, 1 CPU core minimum for Vaanyze

### Check pg_stat_statements

```sql
-- Verify extension is installed
SELECT * FROM pg_extension WHERE extname = 'pg_stat_statements';

-- Check if data is being collected
SELECT count(*) FROM pg_stat_statements;

-- Should return > 0 if queries have been captured
```

---

## Phase 1: Read-Only Validation (48 hours)

### Step 1: Start Vaanyze in Observe Mode

```bash
# Using Docker
docker run -d \
  --name vaanyze \
  -e DATABASE_URL="postgresql://user:pass@host:5432/dbname" \
  -e OPERATION_MODE=observe_only \
  -p 9090:9090 \
  ghcr.io/vaanyze/vaanyze:latest

# Verify it started
docker logs vaanyze
```

### Step 2: Verify Connection and Environment

```bash
# Check health
curl http://localhost:9090/api/v1/health | jq

# Detect environment (will warn if managed DB detected)
curl http://localhost:9090/api/v1/system/environment | jq
```

**Expected output:**
```json
{
  "environment": "SelfManaged",
  "has_pg_stat_statements": true,
  "can_create_index": true,
  "recommended_mode": "safe_auto"
}
```

### Step 3: Review Current Recommendations

```bash
# Get recommendations
curl http://localhost:9090/api/v1/recommendations | jq

# Analyze a specific slow query
curl -X POST http://localhost:9090/api/v1/queries/analyze \
  -H "Content-Type: application/json" \
  -d '{"query": "SELECT * FROM orders WHERE customer_id = $1"}' | jq
```

### Step 4: Validate a Recommendation (No Apply)

Pick a recommendation and validate it without applying:

```bash
curl -X POST http://localhost:9090/api/v1/index/validate \
  -H "Content-Type: application/json" \
  -d '{
    "query": "SELECT * FROM orders WHERE customer_id = $1",
    "table_name": "orders",
    "columns": ["customer_id"]
  }' | jq
```

**Expected output:**
```json
{
  "recommendation_id": "abc123",
  "state": "VALIDATED",
  "validated": true,
  "reason": "Index used by planner, cost improved 75%",
  "cost_improvement_pct": 75.2,
  "index_used": true,
  "diff_summary": "Plan changed: Seq Scan -> Index Scan on orders using vaanyze_idx_orders_abc"
}
```

### Step 5: 48-Hour Observation

Let Vaanyze run for 48 hours collecting data. Check periodically:

```bash
# Check audit log for any issues
curl http://localhost:9090/api/v1/index/audit | jq

# Review recommendation quality
curl http://localhost:9090/api/v1/recommendations?sort=impact | jq
```

**What to evaluate during this phase:**
- [ ] Are recommendations relevant to your actual slow queries?
- [ ] Do EXPLAIN proofs show meaningful improvements?
- [ ] Are there any false positives (recommendations for already-fast queries)?

---

## Phase 2: Single Safe Apply (15 minutes)

**Only proceed if Phase 1 was successful.**

### Step 1: Select a Low-Risk Recommendation

Choose a recommendation that:
- Targets a non-critical table
- Has high cost improvement (>50%)
- Shows "index_used": true in validation

### Step 2: Apply with Verification

```bash
# Apply the validated recommendation
curl -X POST http://localhost:9090/api/v1/index/apply \
  -H "Content-Type: application/json" \
  -d '{
    "recommendation_id": "abc123",
    "triggered_by": "design-partner-pilot",
    "verification_minutes": 30
  }' | jq
```

**Expected response:**
```json
{
  "state": "VERIFYING",
  "index_name": "vaanyze_idx_orders_abc",
  "message": "Index created successfully. Entering verification window.",
  "verification_until": "2025-01-15T10:30:00Z"
}
```

### Step 3: Monitor Verification

```bash
# Check status during verification window
curl http://localhost:9090/api/v1/index/status/abc123 | jq

# List all Vaanyze-managed indexes
curl http://localhost:9090/api/v1/system/indexes | jq
```

### Step 4: Verify Index Usage

After the verification window:

```sql
-- Check if index is being used
SELECT indexrelname, idx_scan, idx_tup_read
FROM pg_stat_user_indexes
WHERE indexrelname LIKE 'vaanyze_idx_%';
```

### Step 5: Manual Rollback (Optional)

If you want to test rollback:

```bash
curl -X POST http://localhost:9090/api/v1/index/rollback \
  -H "Content-Type: application/json" \
  -d '{
    "recommendation_id": "abc123",
    "reason": "Testing rollback functionality",
    "triggered_by": "design-partner"
  }' | jq
```

---

## What Vaanyze Will Do

1. **Collect** - Read query statistics from pg_stat_statements
2. **Analyze** - Identify slow queries and potential index improvements
3. **Validate** - Run EXPLAIN before/after with candidate index
4. **Apply** (if enabled) - CREATE INDEX CONCURRENTLY
5. **Verify** - Monitor idx_scan and query latency
6. **Rollback** (if needed) - DROP INDEX CONCURRENTLY

## What Vaanyze Will NOT Do

- ❌ Modify table data
- ❌ Change PostgreSQL configuration
- ❌ Create indexes without EXPLAIN proof
- ❌ Apply without verification window
- ❌ Make changes during active transactions (uses CONCURRENTLY)
- ❌ Access kernel/eBPF (optional feature, disabled by default)

---

## Emergency Procedures

### Kill Switch

If you need to stop all Vaanyze operations immediately:

```bash
# Activate kill switch
curl -X POST http://localhost:9090/api/v1/system/kill-switch \
  -H "Content-Type: application/json" \
  -d '{"enabled": true, "reason": "Emergency stop"}'
```

### Manual Index Cleanup

```sql
-- List all Vaanyze indexes
SELECT indexname, tablename, indexdef
FROM pg_indexes
WHERE indexname LIKE 'vaanyze_idx_%';

-- Drop a specific index
DROP INDEX CONCURRENTLY IF EXISTS vaanyze_idx_orders_abc;

-- Drop all Vaanyze indexes (use with caution)
DO $$
DECLARE
    idx RECORD;
BEGIN
    FOR idx IN SELECT indexname FROM pg_indexes WHERE indexname LIKE 'vaanyze_idx_%'
    LOOP
        EXECUTE 'DROP INDEX CONCURRENTLY IF EXISTS ' || idx.indexname;
    END LOOP;
END $$;
```

### Orphaned Index Cleanup

If Vaanyze crashed during validation:

```bash
curl -X POST http://localhost:9090/api/v1/system/recover | jq
```

Or manually:
```sql
-- Find orphaned candidate indexes (should never exist in normal operation)
SELECT indexname FROM pg_indexes WHERE indexname LIKE '_vaanyze_candidate_%';

-- Drop them
DROP INDEX IF EXISTS _vaanyze_candidate_abc123;
```

---

## Troubleshooting

### "Permission denied" during validation

```sql
-- Grant required permissions
GRANT SELECT ON pg_stat_statements TO your_user;
GRANT CREATE ON SCHEMA public TO your_user;
```

### "Index not used" in validation

This means PostgreSQL's planner chose not to use the index. Common reasons:
- Table is too small (< 1000 rows)
- Statistics are outdated: `ANALYZE table_name;`
- Query pattern doesn't match index columns

### No recommendations generated

- Check if pg_stat_statements has data: `SELECT count(*) FROM pg_stat_statements`
- Ensure queries have been running for at least 1 hour
- Check thresholds in config (default: queries > 100ms mean time)

---

## Reporting Issues

When reporting issues, please include:

1. Output of `/api/v1/health`
2. Output of `/api/v1/system/environment`
3. Relevant logs: `docker logs vaanyze 2>&1 | tail -100`
4. The specific recommendation ID and query

---

## Success Criteria

A successful design partner evaluation shows:

- [ ] Vaanyze correctly identifies slow queries in your workload
- [ ] EXPLAIN proofs are accurate and meaningful
- [ ] At least one index was applied and verified successfully
- [ ] Index usage (idx_scan) increased after apply
- [ ] No unexpected side effects or errors
- [ ] Rollback works as expected (if tested)

## Next Steps After Pilot

1. Review all recommendations for production applicability
2. Discuss safe_auto mode for continuous optimization
3. Plan integration with your monitoring/alerting stack
4. Consider Slack/webhook notifications for apply events
