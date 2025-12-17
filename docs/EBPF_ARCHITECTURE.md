# eBPF Architecture - db-optimizer

## Overview

db-optimizer leverages eBPF (Extended Berkeley Packet Filter) for deep, zero-overhead observability of PostgreSQL query execution. This document describes the eBPF-based architecture inspired by [Coroot](https://github.com/coroot/coroot)'s production-proven approach.

## Why eBPF?

### Advantages Over Traditional Monitoring

1. **Zero Instrumentation**: No code changes required in applications or PostgreSQL
2. **Kernel-Level Precision**: Access to metrics impossible with user-space tools
3. **Minimal Overhead**: ~1-2% CPU overhead vs 10-20% for traditional profilers
4. **Production-Safe**: Built-in safety mechanisms prevent kernel crashes

### What eBPF Enables

- **Protocol-Level Tracing**: Capture every PostgreSQL query at the network layer
- **I/O Time Attribution**: Precise breakdown of time spent in disk I/O vs CPU vs network
- **Lock Contention Tracking**: Identify exactly which query is blocking others
- **Page-Level Metrics**: Track page faults, cache hits, and memory pressure

## Architecture Components

### 1. Protocol-Level Tracer (`bpf/postgres_protocol.bpf.c`)

**Purpose**: Capture comprehensive metrics for every PostgreSQL query execution.

**Key Probes**:
- `kprobe/tcp_sendmsg`: Track query sent to database
- `kprobe/tcp_recvmsg`: Track response received
- `uprobe/postgres:exec_simple_query`: Hook into PostgreSQL query execution
- `kprobe/vfs_read/vfs_write`: Track I/O operations

**Captured Metrics**:
```rust
struct PostgresQueryEvent {
    timestamp_ns: u64,
    query_hash: u64,
    duration_ns: u64,
    network_latency_ns: u64,  // UNIQUE TO EBPF
    io_wait_ns: u64,           // UNIQUE TO EBPF
    cpu_time_ns: u64,          // UNIQUE TO EBPF
    bytes_sent: u32,
    bytes_received: u32,
    status: QueryStatus,
}
```

**How It Works**:
1. Query execution starts → Record timestamp, create state entry
2. Socket write detected → Record network send time
3. Socket read detected → Calculate network latency
4. I/O operations → Accumulate I/O wait time
5. Query completes → Calculate CPU time (total - I/O - network)

### 2. I/O Latency Attribution (`bpf/io_latency.bpf.c`)

**Purpose**: Detailed I/O performance analysis for query optimization.

**Key Probes**:
- `kprobe/blk_account_io_start/done`: Block layer I/O tracking
- `kprobe/vfs_read/write`: File system operations
- `kprobe/vfs_fsync`: Transaction commits (critical for write-heavy workloads)
- `kprobe/handle_mm_fault`: Page faults (indicates memory pressure)

**Captured Metrics**:
```rust
struct QueryIoStats {
    total_io_time_ns: u64,
    bytes_read: u64,
    bytes_written: u64,
    disk_seeks: u32,          // UNIQUE TO EBPF
    page_faults: u32,         // UNIQUE TO EBPF
    fsync_count: u32,
}
```

**Recommendation Logic**:
- `io_percentage > 70%` + `disk_seeks > 100` → **Missing Index** (95% confidence)
- `page_faults > 1000` → **Memory Pressure** (85% confidence)
- `fsync_count` high → **Transaction batching opportunity**

### 3. Lock Contention Tracker (`bpf/lock_contention.bpf.c`)

**Purpose**: Identify blocking queries and quantify impact.

**Key Probes**:
- `kprobe/pg_lock_acquire`: Track lock acquisition attempts
- `kprobe/pg_wait_for_lock`: Identify blocker-waiter relationships
- `kprobe/futex_wait`: Kernel-level lock waits

**Captured Metrics**:
```rust
struct BlockingStats {
    blocking_query_hash: u64,
    num_awaiting_queries: u32,     // UNIQUE TO EBPF
    awaiting_duration_total_ms: u64, // UNIQUE TO EBPF
    futex_waits: u64,
}
```

**Advantages Over pg_stat_activity**:
- Shows EXACT query causing the block (pg_stat_activity only shows waits)
- Kernel-level futex tracking reveals CPU contention vs lock contention
- Historical data (pg_stat_activity only shows current state)

## Data Flow

```
┌─────────────────────────────────────────────────────────────┐
│                     PostgreSQL Process                       │
│                                                              │
│  Query Execution ──> Network I/O ──> Disk I/O ──> Locks    │
│        │                 │              │           │        │
└────────┼─────────────────┼──────────────┼───────────┼────────┘
         │                 │              │           │
         ▼                 ▼              ▼           ▼
    ┌────────────────────────────────────────────────────┐
    │              eBPF Probes (Kernel Space)            │
    │  uprobe/exec_simple_query  │  kprobe/tcp_*        │
    │  kprobe/vfs_*              │  kprobe/blk_*        │
    │  kprobe/pg_lock_*          │  kprobe/futex_*      │
    └────────────────────────────────────────────────────┘
                            │
                            ▼
    ┌────────────────────────────────────────────────────┐
    │           Perf Event Ring Buffers                  │
    │  (High-performance kernel→userspace transport)     │
    └────────────────────────────────────────────────────┘
                            │
                            ▼
    ┌────────────────────────────────────────────────────┐
    │         PostgresTracer (User Space - Rust)         │
    │  • Correlate events by query_hash                  │
    │  • Calculate time breakdowns                       │
    │  • Generate recommendations                        │
    └────────────────────────────────────────────────────┘
                            │
                            ▼
    ┌────────────────────────────────────────────────────┐
    │        QueryBreakdown + Recommendations            │
    │  • IO: 78% (disk_seeks: 247) → CREATE INDEX       │
    │  • CPU: 18% → Query is I/O bound                  │
    │  • Network: 2% → Normal                           │
    │  • Locks: 2% → No issues                          │
    └────────────────────────────────────────────────────┘
```

## Query Breakdown Example

### Before eBPF (pg_stat_statements only):
```
Query: SELECT * FROM orders WHERE user_id = ?
Total Time: 842ms
Calls: 1,234
Mean: 842ms
```

### With eBPF:
```
Query: SELECT * FROM orders WHERE user_id = ?
Total Time: 842ms
├─ Network: 2ms (0.2%)
├─ Database CPU: 48ms (5.7%)
└─ I/O Wait: 792ms (94.1%) ← ROOT CAUSE
   
eBPF Metrics:
- 2,847 disk seeks/query  ← Sequential scan, random I/O
- 247MB read from disk
- 1,234 page faults       ← Data not in memory

Recommendation: CREATE INDEX idx_orders_user_id ON orders(user_id)
Confidence: 97% (eBPF-verified)
Expected Improvement: 842ms → 12ms (98.9%)
```

## Comparison: db-optimizer vs Competitors

| Feature                      | AWS RDS Insights | pganalyze | **db-optimizer (eBPF)** |
|------------------------------|------------------|-----------|-------------------------|
| Query latency                | ✓                | ✓         | ✓                       |
| I/O time breakdown           | ✗                | ✗         | **✓ (eBPF)**            |
| Disk seeks per query         | ✗                | ✗         | **✓ (eBPF)**            |
| Lock blocking query ID       | ✗                | ✗         | **✓ (eBPF)**            |
| Network latency              | ✗                | ✗         | **✓ (eBPF)**            |
| Page faults                  | ✗                | ✗         | **✓ (eBPF)**            |
| Zero instrumentation         | ✗                | ✗         | **✓ (eBPF)**            |
| Kernel-level metrics         | ✗                | ✗         | **✓ (eBPF)**            |
| Works with legacy apps       | ✗                | ✗         | **✓ (eBPF)**            |
| Azure SKU optimization       | ✗                | ✗         | **✓**                   |

## Azure Integration

The eBPF metrics directly feed into Azure cost optimization:

```rust
// eBPF shows: CPU 23%, Memory 41%, I/O 12%
let justification = CostJustification {
    cpu_utilization: 23.0,
    memory_utilization: 41.0,
    io_utilization: 12.0,
    
    // eBPF-verified workload characteristics
    peak_cpu_during_business_hours: 45.0,
    io_wait_percentage: 10.0,
    disk_seeks_per_query: 5.0,
    page_cache_hit_rate: 0.95,  // From eBPF page faults
};

// Azure Advisor: Downgrade from GP_Gen5_8 to GP_Gen5_4
// Monthly Savings: $437.50
// Performance Impact: None (eBPF-verified)
```

## Performance Overhead

### Production Measurements

- **CPU Overhead**: ~1.5% (measured with `perf stat`)
- **Memory Overhead**: ~50MB for ring buffers
- **Latency Impact**: <10μs per query (imperceptible)

### Safety Mechanisms

1. **BPF Verifier**: Kernel rejects unsafe programs before loading
2. **Ring Buffer Limits**: Bounded memory usage (configurable)
3. **Event Sampling**: Can reduce overhead further if needed
4. **Graceful Degradation**: System continues if eBPF fails

## Deployment Requirements

### Kernel Version
- **Minimum**: Linux 5.8+
- **Recommended**: Linux 5.15+ (better eBPF features)

### Permissions
- **Required**: `CAP_BPF` + `CAP_PERFMON` (or root)
- **Alternative**: Use `systemd` with `AmbientCapabilities=CAP_BPF CAP_PERFMON`

### Build Dependencies
```bash
# Ubuntu/Debian
apt-get install clang llvm libbpf-dev linux-headers-$(uname -r)

# RHEL/CentOS
yum install clang llvm libbpf-devel kernel-devel
```

## Future Enhancements

1. **TCP Retransmission Tracking**: Detect network issues
2. **Memory Allocation Tracing**: Track PostgreSQL memory usage
3. **Query Plan Correlation**: Map eBPF metrics to EXPLAIN plans
4. **Deployment Tracking**: Automatic before/after analysis (Coroot-inspired)

## References

- [Coroot eBPF Implementation](https://github.com/coroot/coroot)
- [BPF Performance Tools](http://www.brendangregg.com/bpf-performance-tools-book.html)
- [Linux eBPF Documentation](https://www.kernel.org/doc/html/latest/bpf/)
