# Known Limitations

## Environment Constraints

### Managed Database Services

Vaanyze's auto-apply features **do not work** with managed database services:

| Service | Limitation |
|---------|------------|
| AWS RDS / Aurora | Cannot create indexes without admin privileges |
| Azure Database for PostgreSQL | Same as above |
| Google Cloud SQL | Same as above |
| Heroku Postgres | Same as above |

**Workaround**: Use `observe_only` mode to get recommendations, then apply them manually or via your infrastructure-as-code pipeline.

### Operating System

- **Linux required for eBPF features** (kernel 5.8+ recommended)
- eBPF features disabled on macOS, Windows
- Non-eBPF features work on any OS that supports PostgreSQL

### PostgreSQL Version

- Minimum: PostgreSQL 12
- Recommended: PostgreSQL 14+
- pg_stat_statements extension must be enabled

## Connection Poolers

### PgBouncer (Transaction Mode)

**Critical Limitation**: When using PgBouncer in transaction mode, Vaanyze's PID-based correlation **breaks completely**. Each transaction may use a different backend PID.

**Symptoms**:
- Query attribution is unreliable
- eBPF metrics may be attributed to wrong queries

**Workarounds**:
- Use session pooling mode (if acceptable for your workload)
- Run Vaanyze against the PostgreSQL instance directly (bypass pooler)
- Accept that correlation accuracy will be lower

### pgcat, Odyssey, etc.

Same limitations as PgBouncer in transaction mode apply to any transaction-mode connection pooler.

## eBPF Limitations

### Kernel Requirements

- Minimum: Linux kernel 5.4
- Recommended: Linux kernel 5.8+ for full BTF support
- BTF (BPF Type Format) required for many features

### Capabilities

Vaanyze requires:
- `CAP_BPF` - Ability to load BPF programs
- `CAP_PERFMON` - Access to performance monitoring

Or run as root (not recommended in production).

### Container Limitations

When running in containers:
- Requires `--privileged` or specific capabilities
- May need host PID namespace (`--pid=host`)
- Some Kubernetes environments restrict BPF

### Stripped Binaries

PostgreSQL binaries from some distributions are stripped (debug symbols removed):
- Lock contention tracing via uprobes will fail
- Gracefully degrades to other metrics

## Query Analysis Limitations

### Queries Not in pg_stat_statements

Vaanyze can only analyze queries captured by pg_stat_statements:
- Very fast queries may be sampled out
- Queries before pg_stat_statements was enabled
- Queries from extensions that don't use standard execution

### Query Normalization

Vaanyze's query hash may not match pg_stat_statements queryid:
- Different normalization algorithms
- May affect correlation in some edge cases

### Prepared Statements

When using prepared statements with connection poolers:
- Statement names are ephemeral
- Correlation may be unreliable

## Auto-Apply Limitations

### CREATE INDEX CONCURRENTLY

Vaanyze uses `CREATE INDEX CONCURRENTLY` which:
- Takes longer than regular CREATE INDEX
- May fail if there are long-running transactions
- Requires waiting for all transactions to complete

### Multi-Column Index Order

Vaanyze recommends columns in the order they appear in queries, which may not always be optimal. The optimal order depends on:
- Selectivity of each column
- Query patterns (equality vs. range)

**Recommendation**: Review multi-column index suggestions carefully.

### Partial Indexes

Currently, Vaanyze does not recommend partial indexes. If you have highly skewed data distributions, consider manual partial index creation.

### Index Types

Vaanyze primarily recommends B-tree indexes. GIN, GiST, and other specialized indexes are supported but less thoroughly optimized.

## Performance Overhead

### Without eBPF

Minimal overhead:
- Periodic queries to pg_stat_* views
- EXPLAIN execution during validation

### With eBPF

Low but measurable overhead:
- BPF programs run on kernel events
- Ring buffer processing
- Sampling helps limit impact

**Estimate**: <1% CPU overhead in most cases. Monitor with Vaanyze's self-metrics.

## High Availability

### Replicas

- Analysis can run against read replicas
- Auto-apply must run against primary
- Replica PIDs differ from primary (affects eBPF correlation)

### Failover

After failover:
- Verification windows may be interrupted
- Applied indexes should persist (DDL replication depends on your HA setup)
- Audit log (in-memory portion) may be lost - use persistent storage

## Kubernetes-Specific

### CNPG Integration

CloudNativePG integration is experimental:
- Sidecar mode works but has limitations
- Plugin mode is under development
- See [CNPG Integration](./CNPG_INTEGRATION.md) for details

### Ephemeral Storage

If Vaanyze's persistence is on ephemeral storage:
- History lost on pod restart
- Consider using PostgreSQL backend for persistence

## What Vaanyze Cannot Do

1. **Fix application bugs** - Slow queries may be fundamental design issues
2. **Replace a DBA** - Complex schema decisions need human judgment  
3. **Work with other databases** - PostgreSQL only (for now)
4. **Guarantee improvements** - EXPLAIN estimates aren't always accurate
5. **Handle all edge cases** - Unusual workloads may confuse heuristics

## Reporting Issues

If you encounter limitations not documented here:
1. Check the GitHub issues for existing reports
2. Include: Vaanyze version, PostgreSQL version, operating system, error messages
3. Redact sensitive information from any logs or queries
