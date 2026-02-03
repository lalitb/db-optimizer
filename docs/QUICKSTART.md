# Vaanyze Quick Start

Get started with Vaanyze in under 5 minutes.

## Requirements

- PostgreSQL 12+ with `pg_stat_statements` extension enabled
- Database user with `SELECT` privileges on `pg_stat_statements` view
- For auto-apply: `CREATE INDEX` and `DROP INDEX` privileges

## Minimal Setup (Read-Only Mode)

The simplest way to run Vaanyze is with just a database URL:

```bash
# Set your database connection
export DATABASE_URL="postgresql://user:password@localhost:5432/mydb"

# Run analysis (read-only, generates recommendations only)
./db-optimizer analyze -d $DATABASE_URL
```

That's it. Vaanyze will:
1. Connect to your database
2. Query `pg_stat_statements` for slow queries
3. Analyze table and index statistics
4. Generate index recommendations
5. Print a report to stdout

## Configuration Options

### Environment Variables

| Variable | Required | Description |
|----------|----------|-------------|
| `DATABASE_URL` | Yes | PostgreSQL connection string |
| `SLACK_WEBHOOK` | No | Slack webhook URL for notifications |
| `RUST_LOG` | No | Log level (debug, info, warn, error) |

### Configuration File (Optional)

For advanced settings, create a `config.yml`:

```yaml
database:
  url: "${DATABASE_URL}"

operation_mode: "observe_only"  # or "safe_auto" for auto-apply

safety:
  protected_schemas:
    - pg_catalog
    - information_schema
```

## Running Modes

### Analyze Mode (One-Shot)

Run analysis once and exit:

```bash
./db-optimizer analyze -d $DATABASE_URL
```

### Agent Mode (Continuous)

Monitor continuously and generate recommendations periodically:

```bash
./db-optimizer agent -d $DATABASE_URL --interval-minutes 15
```

### HTTP Server Mode

Start the REST API for integrations:

```bash
./db-optimizer serve --port 9090 -d $DATABASE_URL
```

## Verify pg_stat_statements

Vaanyze requires `pg_stat_statements` to be enabled. Check if it's available:

```sql
SELECT * FROM pg_extension WHERE extname = 'pg_stat_statements';
```

If not installed:

```sql
CREATE EXTENSION IF NOT EXISTS pg_stat_statements;
```

Note: You may need to add `shared_preload_libraries = 'pg_stat_statements'` to `postgresql.conf` and restart PostgreSQL.

## Next Steps

- [Read-Only Mode](./SAFETY.md) - Learn about safe operation
- [Auto-Apply](./AUTO_APPLY.md) - Enable automatic index creation
- [API Reference](./API.md) - HTTP API documentation
- [Limitations](./LIMITATIONS.md) - Known constraints
