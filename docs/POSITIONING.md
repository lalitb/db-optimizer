# Vaanyze: Technical Positioning

## What Vaanyze Proves Today

Vaanyze demonstrates that index recommendations can be validated before application using PostgreSQL's EXPLAIN planner. Unlike tools that suggest indexes based solely on query patterns or statistics, Vaanyze creates a candidate index, runs EXPLAIN to verify the planner would actually use it, computes the cost improvement, and only then marks the recommendation as validated. This eliminates the "trust me" problem where operators must blindly accept tooling suggestions. Every recommendation includes the before/after query plans as evidence.

## What Vaanyze Automates Safely

Vaanyze automates the complete index lifecycle with safety guarantees:

1. **Detection** - Identifies slow queries using pg_stat_statements statistics
2. **Validation** - Creates temporary candidate index, runs EXPLAIN proof, cleans up
3. **Apply** - Uses CREATE INDEX CONCURRENTLY to avoid table locks
4. **Verify** - Monitors idx_scan counts and query latency during configurable window
5. **Rollback** - Automatically drops indexes that aren't used or cause regression

The automation includes a kill switch, audit logging, and state machine enforcement that prevents invalid operations. Read-only mode is available for environments where any DDL is prohibited.

## What Vaanyze Explicitly Does Not Attempt Yet

Vaanyze does not:

- **Tune PostgreSQL configuration parameters** - The config tuner module exists but is not integrated with the validation/apply pipeline. Configuration changes require different safety guarantees and operational procedures.

- **Rewrite queries** - The query rewriter module exists but operates independently. Query rewrites have different risk profiles and require application-level changes that Vaanyze cannot validate in isolation.

- **Work on managed databases with DDL restrictions** - AWS RDS, Cloud SQL, and similar environments are detected and forced into read-only mode. While analysis works, auto-apply is disabled because these environments have their own index management expectations.

- **Provide kernel-level lock attribution** - The eBPF modules exist but are optional and disabled by default. Lock contention analysis requires Linux kernel 5.4+, CAP_BPF capability, and is not necessary for the core index optimization use case.

- **Replace APM or monitoring tools** - Vaanyze is not a general observability platform. It focuses narrowly on PostgreSQL index optimization and defers broader monitoring to dedicated tools.

---

*This positioning is based on the actual implementation state as of the current release. Claims are limited to capabilities that have test coverage and documented failure modes.*
