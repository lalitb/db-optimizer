# Advanced Index Intelligence - Phase 2 Implementation Guide

**Timeline:** Weeks 15-18 (Month 3)  
**Priority:** ⭐⭐⭐ HIGH  
**Status:** Design Complete, Ready for Implementation  

---

## Executive Summary

This module transforms pg-query-advisor from basic "missing index" detection to sophisticated index engineering that rivals expert DBAs. It analyzes column characteristics, query patterns, and data distributions to recommend optimal index types (BTREE, GIN, GIST, BRIN, HASH), composite indexes, and partial indexes.

**Current Status:** Phase 1 provides basic index recommendations using `pg_stat_user_indexes` and `pg_stat_statements`.

**Phase 2 Goals:**
1. Intelligent index type selection (not just BTREE)
2. Composite index recommendations (multi-column)
3. Partial index suggestions (with WHERE clauses)
4. Covering indexes (with INCLUDE)
5. Cost/benefit analysis for each recommendation

---

## Architecture Overview

```
┌─────────────────────────────────────────────────────────┐
│            Advanced Index Analyzer (NEW)                 │
├─────────────────────────────────────────────────────────┤
│                                                          │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐ │
│  │   Column     │  │    Query     │  │   Index      │ │
│  │ Statistics   │→ │   Pattern    │→ │    Type      │ │
│  │  Collector   │  │   Analyzer   │  │   Selector   │ │
│  └──────────────┘  └──────────────┘  └──────────────┘ │
│         ↓                  ↓                 ↓          │
│  ┌──────────────────────────────────────────────────┐  │
│  │          Recommendation Engine                    │  │
│  ├──────────────────────────────────────────────────┤  │
│  │ • Composite Index Builder                        │  │
│  │ • Partial Index Designer                         │  │
│  │ • Cost/Benefit Calculator                        │  │
│  └──────────────────────────────────────────────────┘  │
│                           ↓                             │
│                  Optimized Index DDL                    │
└─────────────────────────────────────────────────────────┘

Integration with Current Code:
┌─────────────────────────────────────────────┐
│         src/analyzer.rs (EXISTING)          │
│  - Collects pg_stat_statements              │
│  - Identifies slow queries                  │
│  - Detects seq scans                        │
└─────────────────────────────────────────────┘
                    ↓
┌─────────────────────────────────────────────┐
│  src/analyzer/advanced/mod.rs (NEW)         │
│  - Enhanced column analysis                 │
│  - Intelligent index type selection         │
│  - Composite index design                   │
└─────────────────────────────────────────────┘
                    ↓
┌─────────────────────────────────────────────┐
│    src/recommendations.rs (EXISTING)        │
│  - Builds Recommendation struct             │
│  - Generates migration SQL                  │
│  - Calculates cost savings                  │
└─────────────────────────────────────────────┘
```

---

## Module 1: Column Statistics Collector

### File: src/analyzer/advanced/column_stats.rs

**Purpose:** Gather comprehensive statistics about columns to make intelligent indexing decisions.

**Key Data Collected:**
- Data type and characteristics (JSONB, arrays, text, geometry)
- Cardinality (distinct values / total rows)
- Correlation (physical vs logical order)
- Query patterns (LIKE, range scans, JSON operations)
- Value distribution (most common values, histogram)

**Integration with Current Code:**

```rust
// In src/analyzer.rs - ADD new function
impl DatabaseAnalyzer {
    pub async fn analyze_with_advanced_indexes(&self) -> Result<AnalysisReport> {
        // EXISTING: Collect basic stats
        let queries = self.collect_slow_queries().await?;
        let tables = self.collect_table_info().await?;
        let indexes = self.collect_index_usage().await?;
        
        // NEW: Initialize advanced analyzer
        let mut advanced_analyzer = AdvancedIndexAnalyzer::new(self.pool.clone());
        
        // NEW: For each table with issues, run advanced analysis
        let mut advanced_recommendations = Vec::new();
        for table in &tables {
            if self.needs_advanced_analysis(&table) {
                let advanced_rec = advanced_analyzer
                    .analyze_table(&table.schema_name, &table.table_name)
                    .await?;
                advanced_recommendations.push(advanced_rec);
            }
        }
        
        // EXISTING: Generate recommendations
        let mut recommendations = self.generate_recommendations(&queries, &tables, &indexes);
        
        // NEW: Merge advanced recommendations
        recommendations.extend(
            self.convert_advanced_to_recommendations(advanced_recommendations)
        );
        
        Ok(AnalysisReport {
            recommendations,
            // ... rest of report
        })
    }
}
```

**Column Statistics Structure:**

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColumnStats {
    pub schema_name: String,
    pub table_name: String,
    pub column_name: String,
    pub data_type: PostgresType,
    pub total_rows: i64,
    
    // Cardinality and distribution
    pub distinct_values: i64,
    pub null_fraction: f64,
    pub cardinality_ratio: f64,  // distinct / total
    pub correlation: f64,         // -1.0 to 1.0
    
    // Value characteristics  
    pub avg_width: i32,
    pub most_common_values: Vec<String>,
    pub most_common_freqs: Vec<f64>,
    
    // Query pattern flags
    pub has_json_ops: bool,
    pub has_array_ops: bool,
    pub has_text_patterns: bool,
    pub has_geometric_ops: bool,
    pub has_range_queries: bool,
    pub is_sequential: bool,
}
```

**PostgreSQL Queries Used:**

```sql
-- Get pg_stats (already collected by PostgreSQL)
SELECT 
    n_distinct,
    null_frac,
    correlation,
    most_common_vals,
    most_common_freqs,
    histogram_bounds
FROM pg_stats
WHERE tablename = $1 AND attname = $2;

-- Detect query patterns from pg_stat_statements
SELECT query, calls
FROM pg_stat_statements
WHERE query LIKE '%' || $1 || '%'  -- table name
  AND (
    query LIKE '%' || $2 || '->%'     -- JSON ops
    OR query LIKE '%' || $2 || ' LIKE%' -- Text patterns
    OR query LIKE '%' || $2 || ' @>%'   -- Array ops
  );
```

---

## Module 2: Intelligent Index Type Selector

### File: src/analyzer/advanced/index_type_selector.rs

**Purpose:** Choose optimal index type based on column characteristics.

**Decision Tree:**

```
1. Special Types First:
   - JSONB with JSON ops → GIN (jsonb_path_ops)
   - Arrays with array ops → GIN
   - Geometry with spatial ops → GIST
   - tsvector → GIN

2. Text Search Patterns:
   - LIKE/ILIKE patterns → GIN (gin_trgm_ops)
   
3. Sequential Data (Large Tables):
   - High correlation (>0.9) + >1M rows → BRIN
   
4. Cardinality-Based:
   - High cardinality + equality only → HASH
   - Low cardinality + skewed → Partial BTREE
   
5. Default:
   - BTREE (good for equality and range)
```

**Index Type Comparison:**

| Index Type | Best For | Size | Speed | Limitations |
|------------|----------|------|-------|-------------|
| BTREE | Equality, ranges, sorting | Medium | Fast | Default |
| HASH | Equality only | Small | Very fast | No ranges |
| GIN | JSONB, arrays, full-text | Large | Fast for contains | Write overhead |
| GIST | Geometry, ranges, nearest-neighbor | Large | Good | Complex |
| BRIN | Sequential large data | Tiny | Good for ranges | Needs correlation |

**Example Decisions:**

```rust
// Example 1: JSONB column
ColumnStats {
    column_name: "metadata",
    data_type: PostgresType::Jsonb,
    has_json_ops: true,
}
→ GIN index with jsonb_path_ops

// Example 2: Timestamp on large table
ColumnStats {
    column_name: "created_at",
    data_type: PostgresType::Timestamp,
    total_rows: 10_000_000,
    correlation: 0.95,
    is_sequential: true,
}
→ BRIN index (tiny size, good performance)

// Example 3: UUID with equality queries
ColumnStats {
    column_name: "user_id",
    data_type: PostgresType::Uuid,
    cardinality_ratio: 0.99,
    has_range_queries: false,
}
→ HASH index (smaller than BTREE)

// Example 4: Text with LIKE patterns
ColumnStats {
    column_name: "email",
    data_type: PostgresType::Text,
    has_text_patterns: true,  // LIKE queries detected
}
→ GIN index with gin_trgm_ops
```

**Integration Point:**

```rust
// In src/recommendations.rs - ENHANCE existing function
fn recommend_missing_index(
    table: &TableInfo,
    column: &str,
) -> Recommendation {
    // EXISTING: Basic BTREE recommendation
    let basic_ddl = format!(
        "CREATE INDEX CONCURRENTLY idx_{}_{} ON {}.{} ({})",
        table.table_name, column, table.schema_name, table.table_name, column
    );
    
    // NEW: Use advanced type selection if available
    if let Some(column_stats) = self.get_column_stats(table, column) {
        let selector = IndexTypeSelector::new();
        let advanced_rec = selector.select_index_type(&column_stats, &predicates);
        
        return Recommendation {
            issue_type: IssueType::MissingIndex,
            title: format!("Create {} index on {}.{}", 
                advanced_rec.index_type, table.table_name, column),
            description: advanced_rec.reasoning,
            migration_sql: advanced_rec.ddl,
            confidence: advanced_rec.confidence_score,
            // ... rest
        };
    }
    
    // Fallback to basic recommendation
    // ... existing code
}
```

---

## Module 3: Composite Index Designer

### File: src/analyzer/advanced/composite_index.rs

**Purpose:** Identify opportunities for multi-column indexes that are more efficient than multiple single-column indexes.

**Key Insight:** Queries that filter on multiple columns benefit from composite indexes.

**Example:**
```sql
-- Bad: Two separate indexes
CREATE INDEX idx_orders_customer ON orders(customer_id);
CREATE INDEX idx_orders_status ON orders(status);

-- Query uses both columns
SELECT * FROM orders 
WHERE customer_id = 123 AND status = 'pending';
-- Result: Uses only ONE index, scans more rows than necessary

-- Good: Composite index
CREATE INDEX idx_orders_cust_status ON orders(customer_id, status);
-- Result: Both conditions evaluated in index, minimal rows fetched
```

**Column Ordering Rules:**

1. **Equality predicates first** (WHERE col = ?)
2. **Ordered by selectivity** (most selective first)
3. **Range predicates last** (WHERE col > ?)
4. **ORDER BY columns** at the end

**Example Ordering:**
```sql
-- Query
SELECT * FROM orders 
WHERE customer_id = 123      -- Equality, selective
  AND status = 'pending'     -- Equality, less selective
  AND created_at > '2025-01-01'  -- Range
ORDER BY created_at DESC;

-- Optimal index order
CREATE INDEX idx_orders_composite 
  ON orders(customer_id, status, created_at DESC);
  -- customer_id first (most selective equality)
  -- status second (less selective equality)
  -- created_at last (range + ORDER BY)
```

**Detection Algorithm:**

```rust
// Analyze pg_stat_statements for column combinations
SELECT query, calls, mean_exec_time
FROM pg_stat_statements
WHERE query LIKE '%WHERE%customer_id%AND%status%'
ORDER BY calls * mean_exec_time DESC;

// If combination appears frequently (>100 queries)
// AND has high time cost (calls * mean_time)
// → Recommend composite index
```

**Covering Indexes with INCLUDE:**

```sql
-- Query
SELECT customer_id, status, total, created_at
FROM orders
WHERE customer_id = 123 AND status = 'pending';

-- Index with INCLUDE for index-only scan
CREATE INDEX idx_orders_covering
  ON orders(customer_id, status)
  INCLUDE (total, created_at);
  -- Index contains ALL columns needed
  -- PostgreSQL never touches the table
  -- 10x faster for this query pattern
```

---

## Module 4: Partial Index Designer

### File: src/analyzer/advanced/partial_index.rs (integrated in composite_index.rs)

**Purpose:** Create smaller, more efficient indexes with WHERE clauses.

**When to Use Partial Indexes:**

1. **Skewed data distributions** - One value dominates (>30%)
2. **Status columns** - Most rows in one state
3. **Deleted flags** - Most rows not deleted
4. **Active users** - Most users inactive

**Example 1: Skewed Status Column**
```sql
-- Analysis
SELECT status, COUNT(*), COUNT(*) * 100.0 / SUM(COUNT(*)) OVER() as pct
FROM orders
GROUP BY status;

Result:
  status      | count  | pct
  ------------|--------|------
  completed   | 950000 | 95%   ← Dominates
  pending     | 30000  | 3%
  cancelled   | 20000  | 2%

-- Bad: Full index (wastes 95% of space)
CREATE INDEX idx_orders_status ON orders(status);
-- Size: 100MB

-- Good: Partial index (only non-completed)
CREATE INDEX idx_orders_status_active 
  ON orders(status)
  WHERE status != 'completed';
-- Size: 5MB (95% smaller!)
-- Queries for pending/cancelled are just as fast
```

**Example 2: Active Records**
```sql
-- Analysis: 80% of users are inactive
SELECT is_active, COUNT(*)
FROM users
GROUP BY is_active;

Result:
  is_active | count
  ----------|--------
  false     | 800000  (80%)
  true      | 200000  (20%)

-- Partial index for active users only
CREATE INDEX idx_users_active
  ON users(last_login_at)
  WHERE is_active = true;
-- 80% smaller, perfect for queries filtering active users
```

**Detection Logic:**

```rust
// Check most_common_values from pg_stats
if most_common_freqs[0] > 0.3 {  // >30% in one value
    let dominant_value = most_common_values[0];
    
    // Check if queries typically exclude this value
    let excludes_in_queries = analyze_query_patterns()
        .any(|q| q.contains(format!("{} != '{}'", column, dominant_value)));
    
    if excludes_in_queries {
        return Some(PartialIndexRecommendation {
            where_clause: format!("{} != '{}'", column, dominant_value),
            size_reduction_pct: most_common_freqs[0] * 100.0,
        });
    }
}
```

---

## Integration with Current Codebase

### Step 1: Add New Module Structure

```bash
mkdir -p src/analyzer/advanced
touch src/analyzer/advanced/mod.rs
touch src/analyzer/advanced/column_stats.rs
touch src/analyzer/advanced/index_type_selector.rs
touch src/analyzer/advanced/composite_index.rs
```

### Step 2: Update Cargo.toml

```toml
[dependencies]
# EXISTING
sqlx = { version = "0.7", features = ["postgres", "runtime-tokio-native-tls"] }
tokio = { version = "1", features = ["full"] }
serde = { version = "1.0", features = ["derive"] }

# NEW for advanced indexing
regex = "1.10"  # For query pattern extraction
```

### Step 3: Modify src/analyzer.rs

```rust
// ADD at top
mod advanced;
use advanced::AdvancedIndexAnalyzer;

// ADD new method
impl DatabaseAnalyzer {
    /// Enhanced analysis with advanced index intelligence
    pub async fn analyze_with_advanced_indexes(&self) -> Result<AnalysisReport> {
        // Run existing analysis
        let mut report = self.analyze().await?;
        
        // Add advanced index recommendations
        let mut advanced_analyzer = AdvancedIndexAnalyzer::new(self.pool.clone());
        
        for table in &report.tables_analyzed {
            if let Ok(advanced_rec) = advanced_analyzer
                .analyze_table(&table.schema, &table.name)
                .await 
            {
                // Convert to our Recommendation format
                report.recommendations.extend(
                    self.convert_advanced_recommendations(advanced_rec)
                );
            }
        }
        
        Ok(report)
    }
}
```

### Step 4: Update src/main.rs CLI

```rust
// ADD new flag
#[derive(Parser)]
struct Cli {
    // ... existing flags
    
    /// Enable advanced index intelligence (Phase 2)
    #[arg(long, default_value_t = false)]
    advanced_indexes: bool,
}

// UPDATE analyze command
if cli.advanced_indexes {
    let report = analyzer.analyze_with_advanced_indexes().await?;
} else {
    let report = analyzer.analyze().await?;
}
```

---

## Testing Strategy

### Unit Tests

```rust
#[cfg(test)]
mod tests {
    #[test]
    fn test_jsonb_index_selection() {
        let stats = ColumnStats {
            data_type: PostgresType::Jsonb,
            has_json_ops: true,
            ..Default::default()
        };
        
        let selector = IndexTypeSelector::new();
        let rec = selector.select_index_type(&stats, &[]);
        
        assert_eq!(rec.index_type, IndexType::GIN);
        assert_eq!(rec.opclass, Some("jsonb_path_ops".to_string()));
    }
    
    #[test]
    fn test_brin_for_sequential() {
        let stats = ColumnStats {
            total_rows: 10_000_000,
            correlation: 0.95,
            is_sequential: true,
            ..Default::default()
        };
        
        let rec = selector.select_index_type(&stats, &[]);
        
        assert_eq!(rec.index_type, IndexType::BRIN);
        assert!(rec.estimated_size_mb < 1.0);
    }
}
```

### Integration Tests

```sql
-- Test database setup
CREATE TABLE test_orders (
    id SERIAL PRIMARY KEY,
    customer_id UUID NOT NULL,
    status VARCHAR(20) NOT NULL,
    metadata JSONB,
    created_at TIMESTAMP NOT NULL
);

INSERT INTO test_orders (customer_id, status, metadata, created_at)
SELECT 
    gen_random_uuid(),
    CASE WHEN random() < 0.95 THEN 'completed' ELSE 'pending' END,
    '{"test": true}'::jsonb,
    NOW() - (random() * INTERVAL '1 year')
FROM generate_series(1, 1000000);

-- Run analyzer
SELECT analyze_table('public', 'test_orders');

-- Verify recommendations include:
-- 1. GIN index on metadata (JSONB)
-- 2. BRIN index on created_at (sequential)
-- 3. Partial index on status (WHERE status != 'completed')
```

---

## Performance Impact

### Before (Phase 1):
```
Recommendation: "Create index on orders.customer_id"
DDL: CREATE INDEX CONCURRENTLY idx_orders_customer 
     ON orders(customer_id);
Type: BTREE (always)
Size: 100MB
```

### After (Phase 2):
```
Recommendation 1: "Create composite index on orders(customer_id, status)"
DDL: CREATE INDEX CONCURRENTLY idx_orders_composite
     ON orders(customer_id, status)
     INCLUDE (total);
Type: BTREE
Size: 120MB
Benefit: 80% faster than separate indexes, enables index-only scans

Recommendation 2: "Create BRIN index on orders.created_at"
DDL: CREATE INDEX CONCURRENTLY idx_orders_created_brin
     ON orders(created_at) USING brin;
Type: BRIN
Size: 0.5MB (100x smaller than BTREE)
Benefit: 90% space savings, good performance for ranges

Recommendation 3: "Create GIN trigram index on customers.email"
DDL: CREATE INDEX CONCURRENTLY idx_customers_email_trgm
     ON customers(email) USING gin(email gin_trgm_ops);
Type: GIN
Benefit: Enables LIKE '%pattern%' queries (impossible with BTREE)
```

---

## Implementation Checklist

### Week 15-16: Foundation
- [ ] Create `src/analyzer/advanced/` module structure
- [ ] Implement `ColumnStats` collector
  - [ ] Query `pg_stats` for statistics
  - [ ] Analyze `pg_stat_statements` for patterns
  - [ ] Detect data types and characteristics
- [ ] Implement `IndexTypeSelector`
  - [ ] Decision tree for type selection
  - [ ] Cost/benefit estimation
  - [ ] DDL generation
- [ ] Write unit tests for type selection
- [ ] Integration with existing `src/analyzer.rs`

### Week 17: Composite Indexes
- [ ] Implement `CompositeIndexDesigner`
  - [ ] Find column combinations in queries
  - [ ] Optimize column ordering
  - [ ] Calculate composite benefits
- [ ] Add partial index detection
  - [ ] Analyze value distributions
  - [ ] Design WHERE clauses
  - [ ] Estimate size reductions
- [ ] Write tests for composite recommendations

### Week 18: Integration & Polish
- [ ] Integrate with main analyzer
- [ ] Update CLI with `--advanced-indexes` flag
- [ ] Create integration tests
- [ ] Update documentation
- [ ] Performance testing on sample databases
- [ ] Generate example reports showing before/after

---

## Expected Outcomes

### Quantitative Goals
- **90%+ accuracy** in index type selection
- **80%+ detection** of composite index opportunities
- **50%+ size reduction** through BRIN and partial indexes
- **3x more recommendations** compared to Phase 1

### Qualitative Goals
- Recommendations rival expert DBA advice
- Clear explanations for each index type choice
- Actionable DDL that can be executed immediately
- Confidence scores for each recommendation

---

## Example Output

```
Advanced Index Intelligence Analysis
=====================================

Table: public.orders (1,000,000 rows)

[HIGH PRIORITY] Create composite index on (customer_id, status)
  Type: BTREE
  Reasoning: These columns appear together in 1,247 queries (95% of traffic).
            Combined selectivity: 0.0001 (very selective).
            Current: Sequential scan, 1000ms
            After: Index scan, 45ms (95% faster)
  
  DDL:
    CREATE INDEX CONCURRENTLY idx_orders_cust_status
      ON orders(customer_id, status)
      INCLUDE (total, created_at);
  
  Size: 120MB
  Confidence: 95%

[MEDIUM PRIORITY] Create BRIN index on created_at
  Type: BRIN (Block Range Index)
  Reasoning: Column shows strong sequential pattern (correlation: 0.98).
            Table is large (1M rows). BRIN provides 100x size reduction
            with good performance for range queries.
  
  DDL:
    CREATE INDEX CONCURRENTLY idx_orders_created_brin
      ON orders(created_at) USING brin;
  
  Size: 0.5MB (vs 50MB for BTREE)
  Confidence: 90%

[LOW PRIORITY] Create partial index on status
  Type: BTREE with WHERE clause
  Reasoning: Status 'completed' appears in 95% of rows but queries
            typically filter for 'pending' or 'cancelled'.
            Partial index reduces size by 95%.
  
  DDL:
    CREATE INDEX CONCURRENTLY idx_orders_status_active
      ON orders(status)
      WHERE status != 'completed';
  
  Size: 5MB (vs 100MB for full index)
  Confidence: 85%

Summary:
  Total recommendations: 3
  Estimated query improvement: 80%
  Index storage required: 125MB
  Implementation order: 1 → 2 → 3
```

---

## Competitive Advantage

This Advanced Index Intelligence puts you ahead of:

1. **pganalyze** - Only recommends BTREE indexes
2. **pgBadger** - No index recommendations
3. **Aiven** - Has ML but no open-source alternative
4. **Manual DBA work** - Hours of analysis automated

**Your unique selling point:** "Not just 'create an index' - we tell you which TYPE of index, with which columns, in which order, and why."

---

*Ready to begin Week 15 implementation!*
