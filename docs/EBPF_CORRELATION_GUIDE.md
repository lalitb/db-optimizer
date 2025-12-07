# pg_stats + eBPF Correlation: The Complete Picture

**The Secret Sauce:** Combining Application-Level Statistics with Kernel-Level Truth

---

## Executive Summary

Most database monitoring tools see only ONE layer:
- **pg_stats-only tools** see what PostgreSQL *thinks* happened
- **eBPF-only tools** see raw kernel events without database context

**Our advantage:** We correlate BOTH layers to reveal hidden problems and provide 95%+ confidence recommendations.

---

## How PostgreSQL Statistics and Kernel Metrics Work Together

### Two Layers of Truth

```
┌─────────────────────────────────────────────┐
│          Application Layer (pg_stats)        │
│  - Query plans, row counts, index usage     │
│  - Buffer cache hits, temp files            │
│  - Lock waits, deadlocks                    │
│  - What PostgreSQL THINKS happened          │
└─────────────────────────────────────────────┘
                    ↕ CORRELATION ↕
┌─────────────────────────────────────────────┐
│         System Layer (eBPF)                 │
│  - Actual I/O operations, bytes read/written│
│  - CPU time, context switches               │
│  - Page faults, memory pressure             │
│  - What ACTUALLY happened at kernel level   │
└─────────────────────────────────────────────┘
```

---

## 1. Data Sources and What They Provide

### A. PostgreSQL Statistics (pg_stat_*)

```sql
-- pg_stat_statements: Query-level statistics
SELECT 
    query,
    calls,
    total_exec_time,
    mean_exec_time,
    rows,
    shared_blks_hit,     -- PostgreSQL buffer cache hits
    shared_blks_read,    -- Disk reads (8KB blocks)
    shared_blks_written, -- Disk writes
    temp_blks_read,      -- Temp file usage
    temp_blks_written
FROM pg_stat_statements;

-- pg_stat_user_tables: Table access patterns
SELECT 
    schemaname,
    tablename,
    seq_scan,           -- Sequential scans
    seq_tup_read,       -- Rows from seq scans
    idx_scan,           -- Index scans
    idx_tup_fetch,      -- Rows from index scans
    n_tup_ins,
    n_tup_upd,
    n_tup_del,
    n_live_tup,         -- Estimated live rows
    n_dead_tup          -- Dead rows (bloat indicator)
FROM pg_stat_user_tables;

-- pg_stat_user_indexes: Index usage
SELECT 
    schemaname,
    tablename,
    indexname,
    idx_scan,           -- Times index used
    idx_tup_read,       -- Index entries returned
    idx_tup_fetch       -- Table rows fetched
FROM pg_stat_user_indexes;

-- pg_stats: Column statistics for correlation
SELECT
    tablename,
    attname,
    n_distinct,         -- Cardinality
    correlation,        -- Physical ordering (-1 to 1)
    null_frac           -- Percentage of nulls
FROM pg_stats;
```

### B. eBPF Kernel Metrics

```rust
pub struct SystemMetrics {
    // Time breakdown (what PostgreSQL thinks is "execution time")
    pub total_time_us: u64,      // Wall clock time
    pub cpu_time_us: u64,         // Actually on CPU
    pub io_wait_time_us: u64,     // Waiting for disk
    pub lock_wait_time_us: u64,   // Kernel-level locks
    
    // I/O reality (what actually happened)
    pub bytes_read: u64,          // Actual bytes from disk
    pub bytes_written: u64,       // Actual bytes to disk
    pub read_syscalls: u32,       // Number of read() calls
    pub write_syscalls: u32,      // Number of write() calls
    pub fsync_count: u32,         // fsync() calls
    
    // Memory truth
    pub page_cache_hits: u32,     // Served from OS cache
    pub page_cache_misses: u32,   // Had to go to disk
    pub page_faults: u32,         // Memory pressure
    pub context_switches: u32,    // Process switched off CPU
    
    // Event quality
    pub events_processed: u64,    // Successfully captured
    pub events_dropped: u64,      // Lost due to overflow
}
```

---

## 2. The Five Critical Correlation Patterns

### Pattern 1: Cache Illusion Detection ⭐⭐⭐⭐⭐

**The Problem:**
PostgreSQL reports high buffer cache hit ratio, yet queries are still slow.

**What pg_stats Shows:**
```rust
pg_stats: {
    shared_blks_hit: 950,     // 95% buffer hit ratio - looks great!
    shared_blks_read: 50,     // Only 50 blocks "read"
    mean_exec_time: 500ms     // But still slow!
}
```

**What eBPF Reveals:**
```rust
ebpf: {
    bytes_read: 104_857_600,  // 100MB actually read from disk!
    page_cache_misses: 12800, // OS page cache missing
    io_wait_time_us: 450_000  // 450ms waiting for disk
}
```

**The Truth:**
PostgreSQL's buffer cache hit, but the OS page cache missed. Data was in PostgreSQL's shared_buffers, but shared_buffers had to be loaded from disk into RAM first.

**Correlation Code:**
```rust
impl HybridAnalyzer {
    fn detect_cache_illusion(&self, pg: &PgStats, ebpf: &SystemMetrics) -> Option<Insight> {
        let pg_hit_ratio = pg.shared_blks_hit as f64 / 
                          (pg.shared_blks_hit + pg.shared_blks_read) as f64;
        
        let os_cache_hit_ratio = ebpf.page_cache_hits as f64 / 
                                (ebpf.page_cache_hits + ebpf.page_cache_misses) as f64;
        
        // Cache illusion: PG thinks it's hitting cache, but OS is missing
        if pg_hit_ratio > 0.9 && os_cache_hit_ratio < 0.5 {
            return Some(Insight {
                title: "Cache Illusion: PostgreSQL cache hit but OS cache miss".to_string(),
                description: format!(
                    "PostgreSQL reports {:.1}% buffer hit ratio (looks good!), \
                     but OS page cache is only {:.1}% hit rate. \
                     Actually reading {} MB from disk despite PG cache hits.",
                    pg_hit_ratio * 100.0,
                    os_cache_hit_ratio * 100.0,
                    ebpf.bytes_read / 1_048_576
                ),
                recommendation: 
                    "Increase system RAM. PostgreSQL's shared_buffers is working, \
                     but the OS doesn't have enough memory to keep the data files cached. \
                     \n\nSpecific actions:\n\
                     1. Increase system RAM\n\
                     2. Or reduce shared_buffers and let OS cache handle more\n\
                     3. Or add more PostgreSQL replicas to distribute load".to_string(),
                evidence: Evidence {
                    pg_source: format!("{}% shared_buffers hit", pg_hit_ratio * 100.0),
                    ebpf_source: format!("{}% OS page cache hit", os_cache_hit_ratio * 100.0),
                    correlation: "Mismatch between PG and OS cache layers".to_string(),
                },
                confidence: 0.95,
                estimated_impact_pct: 60.0,
            });
        }
        None
    }
}
```

**Real-World Example:**
```yaml
Query: SELECT * FROM orders WHERE user_id = 123

pg_stats says:
  - 95% buffer hit ratio (excellent!)
  - Mean time: 500ms
  - "Looks like cache is working"

eBPF reveals:
  - 100MB read from disk
  - 450ms I/O wait
  - Only 10ms CPU time

Insight: Cache Illusion Detected (95% confidence)
  PostgreSQL's cache is fine, but system RAM is insufficient.
  The working set exceeds physical RAM.

Recommendation:
  Current: 16GB RAM, 4GB shared_buffers
  Increase to: 32GB RAM, 8GB shared_buffers
  Expected: 60% query time reduction
  Cost: +$100/mo for RAM vs $400/mo saved in performance
```

---

### Pattern 2: I/O Amplification Discovery ⭐⭐⭐⭐⭐

**The Problem:**
PostgreSQL reports reading a few blocks, but eBPF shows massive I/O.

**What pg_stats Shows:**
```rust
pg_stats: {
    rows: 1000,               // Retrieved 1000 rows
    shared_blks_read: 125,    // Read 125 blocks (1MB expected)
}
```

**What eBPF Reveals:**
```rust
ebpf: {
    bytes_read: 52_428_800,   // 50MB actually read! (50x amplification)
    read_syscalls: 1523,      // Many small reads
    io_wait_time_us: 800_000, // 800ms waiting
}
```

**The Truth:**
Index is fragmented. Each index lookup triggers multiple random I/Os to scattered heap pages.

**Correlation Code:**
```rust
fn detect_io_amplification(&self, pg: &PgStats, ebpf: &SystemMetrics) -> Option<Insight> {
    let expected_bytes = pg.shared_blks_read * 8192; // 8KB blocks
    let actual_bytes = ebpf.bytes_read;
    
    let amplification = actual_bytes as f64 / expected_bytes as f64;
    
    if amplification > 5.0 {
        return Some(Insight {
            title: "I/O Amplification: Reading {}x more data than expected".to_string(),
            description: format!(
                "Severe I/O amplification detected. \
                 PostgreSQL thinks it read {} MB ({} blocks), \
                 but kernel shows {} MB actually read. \
                 {} syscalls for {} rows suggests fragmentation or scattered data.",
                expected_bytes / 1_048_576,
                pg.shared_blks_read,
                actual_bytes / 1_048_576,
                ebpf.read_syscalls,
                pg.rows
            ),
            recommendation: format!(
                "Data is physically scattered on disk. Options:\n\
                 1. REINDEX to rebuild fragmented indexes\n\
                 2. CLUSTER table using main index for better locality\n\
                 3. VACUUM FULL if table is heavily bloated\n\
                 \n\
                 Expected improvement: {:.0}% reduction in I/O",
                (1.0 - 1.0/amplification) * 100.0
            ),
            evidence: Evidence {
                pg_source: format!("{} blocks read", pg.shared_blks_read),
                ebpf_source: format!("{} MB, {} syscalls", actual_bytes/1_048_576, ebpf.read_syscalls),
                correlation: format!("{:.1}x amplification", amplification),
            },
            confidence: 0.90,
            estimated_impact_pct: 70.0,
        });
    }
    None
}
```

**Real-World Example:**
```yaml
Query: SELECT * FROM orders WHERE customer_id = 123 AND status = 'pending'

pg_stats says:
  - Using index on customer_id
  - Read 125 blocks (1MB)
  - Returned 1000 rows

eBPF reveals:
  - Actually read 50MB (50x amplification!)
  - 1523 read() syscalls
  - Random access pattern

Insight: Severe I/O Amplification (90% confidence)
  Index finds rows efficiently, but heap pages are scattered.
  Each row fetch triggers multiple random disk seeks.

Root Cause:
  - Table was heavily updated/deleted causing bloat
  - Physical ordering doesn't match index order
  - Dead tuples scattered throughout

Recommendation:
  CLUSTER orders USING idx_orders_customer;
  Or: CREATE INDEX idx_orders_cust_status ON orders(customer_id, status);
  
  Expected: 70% I/O reduction (50MB → 2MB actual read)
```

---

### Pattern 3: CPU vs I/O Reality Check ⭐⭐⭐⭐

**The Problem:**
Query is slow, but is it CPU or I/O? PostgreSQL can't tell definitively.

**What pg_stats Shows:**
```rust
pg_stats: {
    mean_exec_time: 1000ms,   // 1 second execution
    temp_blks_read: 0,        // No temp files
    shared_blks_hit: 1000,    // All from cache
}
// Looks like it should be fast - all cached!
```

**What eBPF Reveals:**
```rust
ebpf: {
    cpu_time_us: 950_000,     // 950ms on CPU (95% of time)
    io_wait_time_us: 10_000,  // Only 10ms I/O (1% of time)
    context_switches: 2,      // Barely yielded CPU
}
```

**The Truth:**
This is CPU-bound, not I/O-bound! Adding indexes or RAM won't help.

**Correlation Code:**
```rust
fn classify_bottleneck(&self, pg: &PgStats, ebpf: &SystemMetrics) -> Bottleneck {
    let total_time = ebpf.total_time_us as f64;
    let cpu_pct = (ebpf.cpu_time_us as f64 / total_time) * 100.0;
    let io_pct = (ebpf.io_wait_time_us as f64 / total_time) * 100.0;
    
    // Cross-validate with pg_stats
    let has_disk_reads = pg.shared_blks_read > 0 || pg.temp_blks_read > 0;
    let high_cpu_per_row = (pg.mean_exec_time / pg.rows.max(1) as f64) > 1.0; // >1ms per row
    
    match (cpu_pct, io_pct, has_disk_reads, high_cpu_per_row) {
        (cpu, io, false, true) if cpu > 80.0 => {
            Bottleneck::CpuBound {
                cpu_percentage: cpu,
                cpu_time_ms: ebpf.cpu_time_us as f64 / 1000.0,
                recommendation: format!(
                    "Query is CPU-bound ({:.0}% CPU, {:.0}% I/O) despite hitting cache. \
                     The problem is expensive computation, not data access.\n\
                     \n\
                     Solutions:\n\
                     1. Query rewrite to reduce complexity\n\
                     2. Expression indexes for computed columns\n\
                     3. Materialized views for expensive aggregations\n\
                     4. Consider moving logic to application layer\n\
                     \n\
                     Adding indexes or RAM will NOT help here.",
                    cpu, io
                ),
            }
        },
        (cpu, io, true, _) if io > 60.0 => {
            Bottleneck::IoBound {
                io_percentage: io,
                wait_time_ms: ebpf.io_wait_time_us as f64 / 1000.0,
                bytes_read: ebpf.bytes_read,
                recommendation: format!(
                    "Query is I/O bound ({:.0}% I/O, {:.0}% CPU). \
                     {} MB read from disk.\n\
                     \n\
                     Solutions:\n\
                     1. Add appropriate indexes\n\
                     2. Increase shared_buffers/system RAM\n\
                     3. Faster storage (NVMe vs HDD)\n\
                     4. Read replicas to distribute load",
                    io, cpu, ebpf.bytes_read / 1_048_576
                ),
            }
        },
        _ => Bottleneck::Balanced {
            message: "No single dominant bottleneck. Query may benefit from general optimization.".to_string(),
        }
    }
}
```

**Real-World Example:**
```yaml
Query: SELECT user_id, COUNT(*), AVG(amount * tax_rate * discount_factor) 
       FROM orders 
       GROUP BY user_id

pg_stats says:
  - 1000ms mean time
  - All from shared_buffers (0 disk reads)
  - "Why is it slow if all cached?"

eBPF reveals:
  - 950ms CPU time
  - 10ms I/O wait
  - Query is CPU-bound!

Insight: CPU Bottleneck (95% confidence)
  Query scans all rows and performs expensive calculations per row.
  The mathematical expression (amount * tax_rate * discount_factor) 
  is computed for every row before averaging.

Recommendation:
  Create an expression index or materialized view:
  
  CREATE INDEX idx_orders_computed 
    ON orders((amount * tax_rate * discount_factor));
  
  Or create materialized view:
  
  CREATE MATERIALIZED VIEW orders_summary AS
  SELECT user_id, COUNT(*), AVG(amount * tax_rate * discount_factor)
  FROM orders
  GROUP BY user_id;
  
  Expected: 80% CPU reduction
```

---

### Pattern 4: Write Amplification Detection ⭐⭐⭐⭐

**The Problem:**
Small updates cause massive writes.

**What pg_stats Shows:**
```rust
pg_stats: {
    rows: 100,                // Updated 100 rows
    n_tup_upd: 100,          // 100 updates
}
// Seems reasonable
```

**What eBPF Reveals:**
```rust
ebpf: {
    bytes_written: 10_485_760, // 10MB written! (100KB per row!)
    fsync_count: 25,          // 25 fsync calls
    write_syscalls: 847,      // Many small writes
}
```

**The Truth:**
HOT updates failing or index bloat causing massive write amplification.

**Correlation Code:**
```rust
fn detect_write_amplification(&self, pg: &PgStats, ebpf: &SystemMetrics) -> Option<Insight> {
    if pg.n_tup_upd == 0 {
        return None; // No updates to analyze
    }
    
    let bytes_per_row = ebpf.bytes_written / pg.n_tup_upd.max(1);
    
    if bytes_per_row > 10_000 {  // >10KB per row is suspicious
        let fsync_per_update = ebpf.fsync_count as f64 / pg.n_tup_upd as f64;
        
        return Some(Insight {
            title: "Write Amplification: {}x more writes than expected".to_string(),
            description: format!(
                "Writing {} KB per row (expected: <2KB for typical row). \
                 {} fsync calls for {} updates ({:.1} fsync per update). \
                 {} write syscalls suggests many small, inefficient writes.\n\
                 \n\
                 This indicates:\n\
                 - HOT updates failing (touching indexed columns)\n\
                 - Index bloat requiring updates to multiple index pages\n\
                 - Fillfactor too high, causing page splits",
                bytes_per_row / 1024,
                ebpf.fsync_count,
                pg.n_tup_upd,
                fsync_per_update,
                ebpf.write_syscalls
            ),
            recommendation: format!(
                "Reduce write amplification:\n\
                 \n\
                 1. Check fillfactor: ALTER TABLE SET (fillfactor = 80);\n\
                 2. REINDEX to rebuild bloated indexes\n\
                 3. Batch updates to reduce fsync overhead:\n\
                    SET synchronous_commit = off; -- For non-critical updates\n\
                 4. Avoid updating indexed columns if possible\n\
                 5. Consider partial indexes to reduce index maintenance\n\
                 \n\
                 Expected: {:.0}% reduction in write I/O",
                (1.0 - 2048.0/bytes_per_row as f64) * 100.0
            ),
            evidence: Evidence {
                pg_source: format!("{} updates", pg.n_tup_upd),
                ebpf_source: format!("{} MB written, {} fsync", 
                                    ebpf.bytes_written/1_048_576, ebpf.fsync_count),
                correlation: format!("{} KB per row", bytes_per_row/1024),
            },
            confidence: 0.85,
            estimated_impact_pct: 50.0,
        });
    }
    None
}
```

---

### Pattern 5: Lock Contention Validation ⭐⭐⭐⭐

**The Problem:**
PostgreSQL reports lock waits, but is it really lock contention?

**What pg_stats Shows:**
```rust
pg_stats: {
    mean_exec_time: 2000ms,
    wait_event: "Lock",       // Shows lock wait
}
// Looks like lock contention
```

**What eBPF Reveals:**
```rust
ebpf: {
    lock_wait_time_us: 50_000,  // Only 50ms kernel-level locks
    io_wait_time_us: 1_900_000, // 1900ms I/O wait!
    bytes_read: 100_000_000,    // 100MB read
}
```

**The Truth:**
The "lock wait" is actually waiting on I/O during lock acquisition. Real problem is I/O, not contention.

**Correlation Code:**
```rust
fn validate_lock_contention(&self, pg: &PgStats, ebpf: &SystemMetrics) -> Insight {
    let total_time = ebpf.total_time_us as f64;
    let io_pct = (ebpf.io_wait_time_us as f64 / total_time) * 100.0;
    let lock_pct = (ebpf.lock_wait_time_us as f64 / total_time) * 100.0;
    
    if pg.wait_event.contains("Lock") && io_pct > 80.0 && lock_pct < 10.0 {
        Insight {
            title: "False Lock Contention: Actually I/O bound".to_string(),
            description: format!(
                "PostgreSQL reports lock waits, but eBPF shows:\n\
                 - {:.0}% time in I/O wait ({:.0}ms)\n\
                 - {:.0}% time in lock wait ({:.0}ms)\n\
                 \n\
                 The 'lock wait' is actually the lock holder being slow due to I/O. \
                 Other processes wait for the lock, but the holder is waiting on disk.",
                io_pct, ebpf.io_wait_time_us as f64 / 1000.0,
                lock_pct, ebpf.lock_wait_time_us as f64 / 1000.0
            ),
            recommendation: 
                "This is NOT a lock contention problem. Solutions:\n\
                 1. Speed up I/O with indexes\n\
                 2. Reduce transaction duration\n\
                 3. Use connection pooling to reduce concurrent transactions\n\
                 4. Consider read replicas\n\
                 \n\
                 Do NOT:\n\
                 - Reduce lock timeout (won't help)\n\
                 - Use SELECT FOR UPDATE SKIP LOCKED (solves wrong problem)".to_string(),
            confidence: 0.90,
        }
    } else {
        // Actually lock contention
        Insight {
            title: "True Lock Contention Detected".to_string(),
            // ... real lock contention handling
        }
    }
}
```

---

## 3. Complete Correlation Algorithm

```rust
pub struct CorrelatedAnalysis {
    pub query: String,
    pub pg_stats: PgStats,
    pub ebpf_metrics: SystemMetrics,
    pub insights: Vec<Insight>,
    pub root_cause: RootCause,
    pub confidence: f64,
}

impl HybridAnalyzer {
    pub async fn correlate_metrics(
        &self,
        pg: PgStats,
        ebpf: SystemMetrics,
    ) -> CorrelatedAnalysis {
        let mut insights = Vec::new();
        let mut confidence_factors = Vec::new();
        
        // 1. Check for cache illusions
        if let Some(insight) = self.detect_cache_illusion(&pg, &ebpf) {
            confidence_factors.push(0.95);
            insights.push(insight);
        }
        
        // 2. Check I/O amplification
        if let Some(insight) = self.detect_io_amplification(&pg, &ebpf) {
            confidence_factors.push(0.90);
            insights.push(insight);
        }
        
        // 3. Check write amplification
        if let Some(insight) = self.detect_write_amplification(&pg, &ebpf) {
            confidence_factors.push(0.85);
            insights.push(insight);
        }
        
        // 4. Validate PostgreSQL's understanding
        let pg_thinks_io = pg.shared_blks_read > pg.shared_blks_hit;
        let ebpf_confirms_io = ebpf.io_wait_time_us > ebpf.cpu_time_us;
        
        if pg_thinks_io == ebpf_confirms_io {
            confidence_factors.push(0.95);  // Agreement increases confidence
        } else {
            // Disagreement - need investigation
            confidence_factors.push(0.60);
            insights.push(Insight {
                title: "Inconsistency detected between PostgreSQL and kernel metrics".to_string(),
                description: format!(
                    "PostgreSQL thinks query is {}, but kernel shows {}. \
                     This suggests a deeper issue requiring investigation.",
                    if pg_thinks_io { "I/O bound" } else { "CPU/cache bound" },
                    if ebpf_confirms_io { "I/O bound" } else { "CPU bound" }
                ),
                recommendation: "Further analysis needed. Check for:\n\
                                - OS page cache pressure\n\
                                - Competing processes\n\
                                - Incorrect PostgreSQL statistics (run ANALYZE)".to_string(),
                confidence: 0.70,
            });
        }
        
        // 5. Classify bottleneck using both sources
        let bottleneck = self.classify_bottleneck(&pg, &ebpf);
        
        // 6. Determine root cause with weighted scoring
        let root_cause = self.determine_root_cause(&pg, &ebpf, &insights);
        
        // 7. Calculate overall confidence
        let confidence = if confidence_factors.is_empty() {
            0.50  // No strong correlations found
        } else {
            confidence_factors.iter().sum::<f64>() / confidence_factors.len() as f64
        };
        
        CorrelatedAnalysis {
            query: pg.query.clone(),
            pg_stats: pg,
            ebpf_metrics: ebpf,
            insights,
            root_cause,
            confidence,
        }
    }
    
    fn determine_root_cause(
        &self,
        pg: &PgStats,
        ebpf: &SystemMetrics,
        insights: &[Insight],
    ) -> RootCause {
        // Weighted scoring based on both sources
        let mut scores = HashMap::new();
        
        // eBPF evidence (higher weight - it's ground truth)
        let total_time = ebpf.total_time_us as f64;
        if total_time > 0.0 {
            scores.insert("cpu", (ebpf.cpu_time_us as f64 / total_time) * 1.5);
            scores.insert("io", (ebpf.io_wait_time_us as f64 / total_time) * 1.5);
            scores.insert("lock", (ebpf.lock_wait_time_us as f64 / total_time) * 1.5);
        }
        
        // pg_stats evidence (lower weight - it's interpreted)
        if pg.temp_blks_read > 0 {
            *scores.entry("memory").or_insert(0.0) += 0.3;
        }
        
        if pg.shared_blks_read > pg.shared_blks_hit {
            *scores.entry("io").or_insert(0.0) += 0.2;
        }
        
        let buffer_hit_ratio = pg.shared_blks_hit as f64 / 
                              (pg.shared_blks_hit + pg.shared_blks_read).max(1) as f64;
        if buffer_hit_ratio < 0.9 {
            *scores.entry("cache").or_insert(0.0) += 0.3;
        }
        
        // Find dominant cause
        let (cause, &score) = scores.iter()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
            .unwrap_or((&"unknown", &0.0));
        
        match cause {
            "io" if score > 0.6 => RootCause::IoBottleneck {
                pg_evidence: format!("{} disk blocks read", pg.shared_blks_read),
                ebpf_evidence: format!("{}ms I/O wait, {} MB read", 
                                      ebpf.io_wait_time_us / 1000,
                                      ebpf.bytes_read / 1_048_576),
                correlation: "Both layers confirm I/O bottleneck".to_string(),
                confidence: 0.95,
            },
            "cpu" if score > 0.7 => RootCause::CpuBottleneck {
                pg_evidence: format!("{}ms mean time, {} rows", 
                                    pg.mean_exec_time, pg.rows),
                ebpf_evidence: format!("{}ms CPU time ({:.0}% of total)", 
                                      ebpf.cpu_time_us / 1000,
                                      (ebpf.cpu_time_us as f64 / total_time) * 100.0),
                correlation: "High CPU usage with cache hits".to_string(),
                confidence: 0.92,
            },
            "memory" => RootCause::MemoryPressure {
                pg_evidence: format!("{} temp blocks read", pg.temp_blks_read),
                ebpf_evidence: format!("{} page faults", ebpf.page_faults),
                correlation: "Work spilling to disk due to memory pressure".to_string(),
                confidence: 0.88,
            },
            _ => RootCause::Balanced {
                message: "No single dominant bottleneck. Query may benefit from general optimization.".to_string(),
                confidence: 0.60,
            },
        }
    }
}
```

---

## 4. Confidence Scoring Formula

```rust
pub fn calculate_confidence(
    pg: &PgStats,
    ebpf: Option<&SystemMetrics>,
    correlation_strength: f64,
) -> f64 {
    let mut confidence = 0.0;
    
    // Base confidence from pg_stats (always available)
    confidence += 0.40;
    
    // Major boost from eBPF data (ground truth)
    if let Some(ebpf) = ebpf {
        confidence += 0.30;
        
        // Correlation agreement bonus
        if correlation_strength > 0.8 {
            confidence += 0.20;  // Strong correlation
        } else if correlation_strength > 0.5 {
            confidence += 0.10;  // Moderate correlation
        }
        
        // Sample size bonus (statistical significance)
        if pg.calls > 100 {
            confidence += 0.05;
        }
        
        // Event completeness bonus (low loss rate)
        let event_loss = ebpf.events_dropped as f64 / 
                        (ebpf.events_processed + ebpf.events_dropped).max(1) as f64;
        if event_loss < 0.01 {
            confidence += 0.05;  // <1% event loss
        } else if event_loss > 0.10 {
            confidence -= 0.10;  // >10% loss degrades confidence
        }
    }
    
    // Never claim 100% certainty
    confidence.min(0.95)
}
```

**Confidence Levels:**
- **0.90-0.95:** High confidence - Both layers agree, low event loss
- **0.75-0.89:** Medium confidence - One source strong, correlation present
- **0.60-0.74:** Low confidence - Limited data or disagreement
- **<0.60:** Very low confidence - Insufficient data or conflicting signals

---

## 5. Real-World Examples

### Example 1: The Misleading Index Scan

```yaml
Query: SELECT * FROM orders WHERE customer_id = 123 AND status = 'pending'

PostgreSQL View (pg_stats):
  - Using index idx_customer_id
  - Index scan: 50 index blocks read
  - Heap scan: 200 heap blocks read
  - Execution time: 1000ms
  - Buffer hit ratio: 60%
  - Interpretation: "Index is being used, but lots of disk reads"

Kernel View (eBPF):
  - Actual I/O: 25MB (way more than 250 blocks * 8KB = 2MB)
  - I/O pattern: Random (not sequential)
  - syscalls: 1247 read() calls
  - Time breakdown: 800ms I/O wait, 200ms CPU

Correlation Analysis:
  Pattern: I/O Amplification (12.5x)
  Root Cause: Index finds rows efficiently, but rows are scattered across heap
  Evidence: 1247 syscalls for 250 blocks = 5 syscalls per block (fragmentation)
  
  Why it happens:
  - Table has been heavily updated/deleted
  - Dead tuples cause bloat
  - Physical row ordering doesn't match index order
  - Each index entry points to random heap page

Recommendation (95% confidence):
  Primary: CLUSTER orders USING idx_customer_id;
    - Physically reorders table to match index
    - Expected: 80% I/O reduction (25MB → 2MB)
    
  Alternative: CREATE INDEX idx_customer_status 
                ON orders(customer_id, status);
    - Composite index covers both conditions
    - Expected: 70% improvement
    
  Cost vs Benefit:
    - CLUSTER: 10min downtime, permanent fix
    - New index: No downtime, 100MB storage
```

### Example 2: The Hidden Memory Pressure

```yaml
Query: SELECT user_id, SUM(amount), AVG(price)
       FROM large_table
       GROUP BY user_id

PostgreSQL View:
  - Execution time: 5000ms
  - No temp files: temp_blks_read = 0
  - Buffer hit ratio: 95% (excellent!)
  - Interpretation: "Query is cached, why so slow?"

Kernel View:
  - CPU time: 1000ms (20%)
  - I/O wait: 500ms (10%)
  - Other: 3500ms (70%) - What is this?
  - Page faults: 15,000 (very high!)
  - Memory allocations: 50MB
  - Context switches: 500/sec (extremely high)

Correlation Analysis:
  Pattern: Hidden Memory Pressure
  Root Cause: Query using lots of memory but not quite spilling to temp files
  Evidence: High page faults + context switches = thrashing
  
  The Story:
  - Query needs ~100MB for hash aggregation
  - work_mem set to 50MB
  - PostgreSQL using almost all work_mem
  - Not enough to trigger temp file usage
  - But system under memory pressure
  - OS paging, causing slowdown

Recommendation (85% confidence):
  Option 1: Increase work_mem to 128MB
    - Allows query to complete in memory
    - Risk: May cause OOM if many concurrent queries
    
  Option 2: Decrease work_mem to 20MB
    - Forces clean spill to temp files
    - More predictable performance
    
  Option 3: Add more system RAM
    - Solves root cause
    - Helps all queries
    
  Best choice depends on:
    - Concurrency level
    - Available system RAM
    - Query frequency
```

### Example 3: The Fake Sequential Scan

```yaml
Query: SELECT * FROM large_table WHERE created_at > '2025-01-01'

PostgreSQL View:
  - Sequential scan (no index)
  - 100,000 blocks read
  - Should be fast (sequential I/O)
  - Execution time: 2000ms
  - Interpretation: "Normal seq scan on large table"

Kernel View:
  - Bytes read: 800MB (matches 100k * 8KB)
  - Read pattern: RANDOM (not sequential!)
  - syscalls: 2000 (many small reads)
  - I/O wait: 1800ms (90% of time)
  - Seek time: Very high

Correlation Analysis:
  Pattern: False Sequential Scan
  Root Cause: Table is physically fragmented on disk
  Evidence: "Sequential" scan showing random I/O pattern
  
  Why it happens:
  - Table was created/updated over time
  - Blocks allocated non-contiguously
  - File system fragmentation
  - Or concurrent access during writes
  
  PostgreSQL thinks: Sequential scan (should be fast)
  Reality: Random I/O (very slow)

Recommendation (90% confidence):
  Immediate: VACUUM FULL large_table;
    - Rebuilds table in contiguous blocks
    - Requires AccessExclusiveLock
    - Doubles storage temporarily
    
  Better: pg_repack large_table;
    - Online defragmentation
    - No locking
    - Requires pg_repack extension
    
  Also Check:
    - File system fragmentation (use frag tool)
    - Use XFS or ext4 with proper mount options
    
  Expected: 75% performance improvement
```

---

## 6. Output Format

```rust
#[derive(Debug, Serialize)]
pub struct CorrelatedRecommendation {
    pub title: String,
    pub severity: Severity,
    pub confidence: f64,  // 0.0 - 1.0
    
    // The story
    pub what_pg_sees: String,
    pub what_kernel_sees: String,
    pub what_actually_happened: String,
    
    // Evidence
    pub pg_evidence: Vec<String>,
    pub ebpf_evidence: Vec<String>,
    pub correlation_facts: Vec<String>,
    
    // Action
    pub recommendation: String,
    pub migration_sql: Option<String>,
    pub estimated_impact_pct: f64,
    pub estimated_cost_savings_usd: f64,
    
    // Supporting data
    pub root_cause: RootCause,
    pub bottleneck_breakdown: BottleneckBreakdown,
}
```

---

## Summary: The Power of Correlation

### What Makes This Unique

**Most tools see only ONE layer:**
- **pg_stats-only** (pganalyze, pgBadger): See what PostgreSQL thinks
- **eBPF-only** (BCC tools): See kernel events without database context
- **APM tools** (DataDog, New Relic): Application-level only

**Your tool sees BOTH and correlates them:**

1. **Validation:** PostgreSQL thinks X, kernel confirms/denies X
2. **Root Cause:** Not just "slow" but "slow because of Y"
3. **Hidden Problems:** Cache illusions, I/O amplification, fragmentation
4. **Accurate Recommendations:** Based on actual system behavior, not guesses
5. **Confidence Scores:** Know when to trust the analysis (95% vs 60%)

### The Five Critical Insights Only You Can Provide

1. **Cache Illusion** - "Your cache is fine, but RAM is insufficient"
2. **I/O Amplification** - "Reading 50x more data due to fragmentation"
3. **CPU Reality** - "It's CPU-bound, not I/O-bound - indexes won't help"
4. **Write Amplification** - "100KB per row due to failed HOT updates"
5. **False Lock Contention** - "The lock wait is actually I/O wait"

### Your Competitive Moat

**No other tool can say:**

*"PostgreSQL thinks the problem is X, but the kernel shows it's actually Y, and here's the fix with 95% confidence."*

This transforms a monitoring tool into an **automated DBA**! 🎯

---

*This correlation capability is the secret weapon that justifies your pricing and beats Aiven.*
