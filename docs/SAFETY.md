# Safety and Rollback Behavior

Vaanyze is designed with safety as the primary concern. This document explains the safety mechanisms and how rollback works.

## Trust Model

**Core Principle:** Recommendations are only auto-applied if EXPLAIN proof shows the index will actually be used by the query planner AND provides meaningful cost improvement.

### The Validation Pipeline

Before any index is created in production, Vaanyze:

1. **Creates a Candidate Index** - A temporary index with a special prefix (`_vaanyze_candidate_`) is created to test the recommendation
2. **Runs EXPLAIN Before/After** - The query plan is captured before and after the candidate index exists
3. **Validates Index Usage** - Checks if the query planner actually uses the new index
4. **Measures Cost Improvement** - Calculates the estimated cost reduction
5. **Cleans Up** - The candidate index is dropped regardless of outcome

Only if validation passes (index used + cost improved >= threshold) will the recommendation be considered for auto-apply.

### Validation Outcome Codes

| Code | Meaning | Action |
|------|---------|--------|
| `INDEX_USED` | Index is used and cost improved | Can auto-apply |
| `NOT_USED` | Planner didn't use the index | Do not apply |
| `NO_COST_IMPROVEMENT` | Index used but cost didn't improve enough | Do not apply |
| `QUERY_NOT_EXPLAINABLE` | Can't run EXPLAIN on this query | Manual review |
| `PERMISSION_DENIED` | Insufficient privileges | Check permissions |
| `TIMEOUT` | CREATE INDEX timed out | May need longer timeout |

## Mode Enforcement

Vaanyze enforces operation modes through a centralized `OperationGuard`. All destructive operations (CREATE/DROP INDEX) must pass through this guard.

### Kill Switch

An emergency kill switch is available to immediately stop all apply/rollback operations:

```bash
# Activate kill switch
curl -X POST http://localhost:9090/api/v1/system/kill-switch \
  -H "Content-Type: application/json" \
  -d '{"enabled": true, "reason": "Emergency stop"}'

# Deactivate
curl -X POST http://localhost:9090/api/v1/system/kill-switch \
  -H "Content-Type: application/json" \
  -d '{"enabled": false}'
```

When kill switch is active:
- All apply requests return HTTP 403 Forbidden
- All rollback requests return HTTP 403 Forbidden
- Blocked attempts are logged to audit

### Managed Database Enforcement

Vaanyze automatically detects managed database environments:
- AWS RDS
- AWS Aurora
- Google Cloud SQL
- Azure PostgreSQL
- Heroku

On these environments, Vaanyze **forces read-only mode** automatically. Even if you set `safe_auto` mode, apply/rollback operations will be blocked with a clear error:

```json
{
  "error": "Forbidden: Operation blocked: Managed database environment. Effective mode: READ_ONLY (forced)"
}
```

This prevents accidental modifications on managed databases where you may not have full control.

### Mode Priority

The effective mode is determined by (highest to lowest priority):

1. **Kill switch** - If active, always READ_ONLY
2. **Environment** - Managed DB forces READ_ONLY
3. **User configuration** - Your configured mode

### Checking Current Mode

```bash
curl http://localhost:9090/api/v1/health
```

Response includes:
```json
{
  "operation_guard": {
    "effective_mode": "READ_ONLY",
    "configured_mode": "safe_auto",
    "kill_switch_active": false,
    "environment_forces_read_only": true,
    "destructive_operations_allowed": false,
    "is_forced": true,
    "force_reason": "Managed environment: AwsRds"
  }
}
```

## Operation Modes

### observe_only (Default)

- Collects statistics
- Generates recommendations
- Runs EXPLAIN validation
- **Never creates or drops indexes**

This is the safest mode and recommended for initial deployment.

### safe_auto

- All of `observe_only` plus:
- **Only applies validated recommendations** (EXPLAIN proof passed)
- Uses `CREATE INDEX CONCURRENTLY` (no table locks)
- Enters verification window after apply
- Automatically rolls back on detected issues

### aggressive (Use with Caution)

- More relaxed thresholds
- Shorter verification windows
- Not recommended for production without extensive testing

## State Machine

Every recommendation goes through a defined lifecycle:

```
NEW → VALIDATED → APPLIED → VERIFYING → STABLE
         ↓           ↓          ↓
       FAILED    FAILED    ROLLED_BACK
         ↓
      REJECTED
```

- **NEW**: Recommendation generated, not yet validated
- **VALIDATED**: EXPLAIN proof passed
- **APPLIED**: Index created in database
- **VERIFYING**: Monitoring for regressions (configurable window, default 30 min)
- **STABLE**: Verification passed, index is permanent
- **FAILED**: Validation or apply failed
- **ROLLED_BACK**: Verification failed, index was dropped
- **REJECTED**: Manually rejected by user

## Verification Window

After an index is created, Vaanyze monitors for issues during the verification window (default: 30 minutes):

### What Gets Monitored

1. **Index Usage** (`pg_stat_user_indexes.idx_scan`)
   - If index scans stay at ~0 after window, index may be unused
   - Triggers rollback: index was created but planner isn't using it

2. **Query Performance** (if available)
   - Total execution time delta from `pg_stat_statements`
   - Detects if the recommendation made things worse

### Rollback Triggers

Automatic rollback occurs if:
- Index remains unused after verification window
- Performance regression detected (configurable threshold, e.g., p99 +10%)
- Error during verification checks

### Rollback Action

```sql
DROP INDEX CONCURRENTLY <index_name>;
```

All rollback events are:
- Logged to audit log
- Sent to Slack (if configured)
- Recorded with reason and metrics snapshot

## Protected Resources

Vaanyze will never modify:

- System schemas (`pg_catalog`, `information_schema`)
- Explicitly protected tables (configurable)
- Tables during active transactions (detected via `pg_locks`)

## Index Naming

Indexes are created with deterministic names:

- **Candidate indexes**: `_vaanyze_candidate_<hash>` (temporary, for validation)
- **Production indexes**: `idx_<table>_<columns>_<hash>` (permanent)

The hash ensures idempotency - the same recommendation always generates the same index name.

## Audit Trail

Every action is logged:

- Validation attempts and outcomes
- Apply attempts and results
- Rollback triggers and reasons
- Manual approvals/rejections

Access via API: `GET /api/v1/index/audit`

## Recommendations for Production

1. **Start with `observe_only` mode** - Run for at least a week to build trust
2. **Review recommendations manually** - Understand what Vaanyze suggests
3. **Enable `safe_auto` gradually** - Start with one low-traffic table
4. **Configure Slack notifications** - Stay informed of all actions
5. **Set conservative thresholds** - Higher minimum improvement, longer verification windows
6. **Monitor after enabling** - Watch for any unexpected behavior
