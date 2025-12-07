use super::column_stats::{ColumnStats, PostgresType};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub enum IndexType {
    BTree, // Default, good for equality and range
    Hash,  // Equality only, smaller size
    GIN,   // Full text, JSONB, arrays
    GiST,  // Geometric, full text, nearest-neighbor
    BRIN,  // Block range, good for large sequential data
}

impl IndexType {
    pub fn as_str(&self) -> &str {
        match self {
            IndexType::BTree => "BTREE",
            IndexType::Hash => "HASH",
            IndexType::GIN => "GIN",
            IndexType::GiST => "GIST",
            IndexType::BRIN => "BRIN",
        }
    }

    pub fn as_sql(&self) -> &str {
        match self {
            IndexType::BTree => "btree",
            IndexType::Hash => "hash",
            IndexType::GIN => "gin",
            IndexType::GiST => "gist",
            IndexType::BRIN => "brin",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexRecommendation {
    pub index_type: IndexType,
    pub columns: Vec<String>,
    pub opclass: Option<String>, // Operator class (e.g., gin_trgm_ops)
    pub where_clause: Option<String>, // For partial indexes
    pub confidence_score: f64,
    pub reasoning: String,
    pub estimated_size_mb: f64,
    pub estimated_improvement_pct: f64,
    pub ddl: String,
}

pub struct IndexTypeSelector {
    // Thresholds for decision making
    high_cardinality_threshold: f64,
    low_cardinality_threshold: f64,
    sequential_correlation_threshold: f64,
    large_table_threshold: i64,
}

impl IndexTypeSelector {
    pub fn new() -> Self {
        Self {
            high_cardinality_threshold: 0.95,
            low_cardinality_threshold: 0.01,
            sequential_correlation_threshold: 0.9,
            large_table_threshold: 1_000_000,
        }
    }

    /// Select optimal index type based on column statistics
    pub fn select_index_type(
        &self,
        stats: &ColumnStats,
        _predicates: &[QueryPredicate],
    ) -> IndexRecommendation {
        let decision = self.make_decision(stats);
        let ddl = self.generate_ddl(&decision, stats);

        IndexRecommendation {
            index_type: decision.index_type,
            columns: vec![stats.column_name.clone()],
            opclass: decision.opclass,
            where_clause: decision.where_clause,
            confidence_score: decision.confidence,
            reasoning: decision.reasoning,
            estimated_size_mb: self.estimate_index_size(stats, &decision.index_type),
            estimated_improvement_pct: self.estimate_improvement(stats, &decision.index_type),
            ddl,
        }
    }

    fn make_decision(&self, stats: &ColumnStats) -> IndexDecision {
        // Decision tree for index type selection

        // 1. Check for special data types first
        if let Some(decision) = self.check_special_types(stats) {
            return decision;
        }

        // 2. Check for text search patterns
        if stats.has_text_patterns {
            return self.decide_text_index(stats);
        }

        // 3. Check for sequential data patterns (BRIN candidate)
        if stats.is_sequential && stats.total_rows > self.large_table_threshold {
            return self.decide_brin_index(stats);
        }

        // 4. Check cardinality for Hash vs BTree
        if stats.cardinality_ratio > self.high_cardinality_threshold && !stats.has_range_queries {
            return self.decide_high_cardinality_index(stats);
        }

        // 5. Low cardinality might benefit from partial index
        if stats.cardinality_ratio < self.low_cardinality_threshold {
            return self.decide_low_cardinality_index(stats);
        }

        // 6. Default to BTree
        self.default_btree_decision(stats)
    }

    fn check_special_types(&self, stats: &ColumnStats) -> Option<IndexDecision> {
        match &stats.data_type {
            PostgresType::Jsonb | PostgresType::Json if stats.has_json_ops => {
                Some(IndexDecision {
                    index_type: IndexType::GIN,
                    opclass: Some("jsonb_path_ops".to_string()),
                    confidence: 0.95,
                    reasoning: format!(
                        "GIN index recommended for JSONB column '{}' with JSON operations. \
                         Using jsonb_path_ops for smaller index size and faster containment queries.",
                        stats.column_name
                    ),
                    where_clause: None,
                })
            }

            PostgresType::Array(_) if stats.has_array_ops => {
                Some(IndexDecision {
                    index_type: IndexType::GIN,
                    opclass: None,
                    confidence: 0.9,
                    reasoning: format!(
                        "GIN index recommended for array column '{}'. \
                         Enables efficient array containment queries (ANY, @>, <@).",
                        stats.column_name
                    ),
                    where_clause: None,
                })
            }

            PostgresType::Geometry | PostgresType::Point if stats.has_geometric_ops => {
                Some(IndexDecision {
                    index_type: IndexType::GiST,
                    opclass: None,
                    confidence: 0.95,
                    reasoning: format!(
                        "GiST index recommended for geometric column '{}'. \
                         Supports spatial operations and nearest-neighbor searches.",
                        stats.column_name
                    ),
                    where_clause: None,
                })
            }

            PostgresType::Tsvector => {
                Some(IndexDecision {
                    index_type: IndexType::GIN,
                    opclass: None,
                    confidence: 0.95,
                    reasoning: format!(
                        "GIN index recommended for tsvector column '{}'. \
                         Optimized for full-text search queries.",
                        stats.column_name
                    ),
                    where_clause: None,
                })
            }

            _ => None,
        }
    }

    fn decide_text_index(&self, stats: &ColumnStats) -> IndexDecision {
        IndexDecision {
            index_type: IndexType::GIN,
            opclass: Some("gin_trgm_ops".to_string()),
            confidence: 0.85,
            reasoning: format!(
                "GIN trigram index recommended for text column '{}' with LIKE/ILIKE patterns. \
                 Enables fast pattern matching including wildcards (%pattern%). \
                 Requires pg_trgm extension.",
                stats.column_name
            ),
            where_clause: None,
        }
    }

    fn decide_brin_index(&self, stats: &ColumnStats) -> IndexDecision {
        IndexDecision {
            index_type: IndexType::BRIN,
            opclass: None,
            confidence: 0.9,
            reasoning: format!(
                "BRIN index recommended for column '{}' on large table ({} rows). \
                 Column shows strong sequential pattern (correlation: {:.2}). \
                 BRIN provides 100x smaller index size with good performance for range queries.",
                stats.column_name, stats.total_rows, stats.correlation
            ),
            where_clause: None,
        }
    }

    fn decide_high_cardinality_index(&self, stats: &ColumnStats) -> IndexDecision {
        IndexDecision {
            index_type: IndexType::Hash,
            opclass: None,
            confidence: 0.75,
            reasoning: format!(
                "Hash index recommended for high cardinality column '{}' ({:.1}% unique) \
                 with equality-only queries. Smaller than BTREE but doesn't support range scans.",
                stats.column_name,
                stats.cardinality_ratio * 100.0
            ),
            where_clause: None,
        }
    }

    fn decide_low_cardinality_index(&self, stats: &ColumnStats) -> IndexDecision {
        // Check if we should recommend partial index
        let where_clause = if !stats.most_common_values.is_empty()
            && !stats.most_common_freqs.is_empty()
            && stats.most_common_freqs[0] > 0.3
        {
            let most_common = &stats.most_common_values[0];
            Some(format!("{} != '{}'", stats.column_name, most_common))
        } else {
            None
        };

        let reasoning = if let Some(ref clause) = where_clause {
            format!(
                "Partial BTREE index recommended for low cardinality column '{}'. \
                 Dominant value appears in {:.1}% of rows. \
                 WHERE clause '{}' reduces index size by ~{:.0}%.",
                stats.column_name,
                stats.most_common_freqs[0] * 100.0,
                clause,
                stats.most_common_freqs[0] * 100.0
            )
        } else {
            format!(
                "BTREE index recommended for column '{}' with {:.1}% cardinality.",
                stats.column_name,
                stats.cardinality_ratio * 100.0
            )
        };

        IndexDecision {
            index_type: IndexType::BTree,
            opclass: None,
            confidence: if where_clause.is_some() { 0.8 } else { 0.7 },
            reasoning,
            where_clause,
        }
    }

    fn default_btree_decision(&self, stats: &ColumnStats) -> IndexDecision {
        IndexDecision {
            index_type: IndexType::BTree,
            opclass: None,
            confidence: 0.7,
            reasoning: format!(
                "Standard BTREE index recommended for column '{}'. \
                 Good general-purpose index for equality and range queries. \
                 Column has {:.1}% cardinality.",
                stats.column_name,
                stats.cardinality_ratio * 100.0
            ),
            where_clause: None,
        }
    }

    fn estimate_index_size(&self, stats: &ColumnStats, index_type: &IndexType) -> f64 {
        let base_size = match index_type {
            IndexType::BTree => {
                // BTree: ~(rows * (key_size + 8)) / 0.7 (fill factor)
                let key_size = stats.avg_width as f64;
                (stats.total_rows as f64 * (key_size + 8.0)) / 0.7
            }
            IndexType::Hash => {
                // Hash: ~(rows * 8) / 0.75
                (stats.total_rows as f64 * 8.0) / 0.75
            }
            IndexType::GIN => {
                // GIN: Highly variable, estimate 3x column size for JSONB
                stats.total_rows as f64 * stats.avg_width as f64 * 3.0
            }
            IndexType::GiST => {
                // GiST: ~2x BTree size
                let key_size = stats.avg_width as f64;
                (stats.total_rows as f64 * (key_size + 8.0)) / 0.7 * 2.0
            }
            IndexType::BRIN => {
                // BRIN: Tiny - ~128 rows per block
                (stats.total_rows as f64 / 128.0) * 64.0
            }
        };

        // Convert to MB
        base_size / (1024.0 * 1024.0)
    }

    fn estimate_improvement(&self, stats: &ColumnStats, index_type: &IndexType) -> f64 {
        // Estimate query improvement percentage
        match index_type {
            IndexType::BTree => {
                // Log(n) lookup vs full scan
                if stats.total_rows < 100 {
                    return 50.0; // Small table, modest improvement
                }
                let scan_cost = stats.total_rows as f64;
                let index_cost = (stats.total_rows as f64).log2();
                ((scan_cost - index_cost) / scan_cost * 100.0).min(99.0)
            }
            IndexType::Hash => {
                // O(1) for equality
                if stats.total_rows > 1000 {
                    95.0
                } else {
                    80.0
                }
            }
            IndexType::GIN => {
                // Excellent for contained searches
                85.0
            }
            IndexType::BRIN => {
                // Good for range scans on large tables
                if stats.total_rows > 10_000_000 {
                    90.0
                } else {
                    70.0
                }
            }
            IndexType::GiST => 75.0,
        }
    }

    fn generate_ddl(&self, decision: &IndexDecision, stats: &ColumnStats) -> String {
        let mut ddl = String::new();

        // Index name
        let index_name = format!(
            "idx_{}_{}_{}",
            stats.table_name,
            stats.column_name,
            decision.index_type.as_sql()
        );

        ddl.push_str(&format!("CREATE INDEX CONCURRENTLY {}\n", index_name));
        ddl.push_str(&format!("  ON {}.{}", stats.schema_name, stats.table_name));
        ddl.push_str(&format!(" USING {}", decision.index_type.as_sql()));

        // Columns with operator class if specified
        ddl.push_str(&format!(" ({}", stats.column_name));
        if let Some(opclass) = &decision.opclass {
            ddl.push_str(&format!(" {}", opclass));
        }
        ddl.push_str(")");

        // WHERE clause for partial index
        if let Some(where_clause) = &decision.where_clause {
            ddl.push_str(&format!("\n  WHERE {}", where_clause));
        }

        ddl.push_str(";");

        ddl
    }
}

impl Default for IndexTypeSelector {
    fn default() -> Self {
        Self::new()
    }
}

struct IndexDecision {
    index_type: IndexType,
    opclass: Option<String>,
    confidence: f64,
    reasoning: String,
    where_clause: Option<String>,
}

#[derive(Debug, Clone)]
pub enum QueryPredicate {
    Equality(String),
    Range(String, String),
    Like(String),
    ILike(String),
    IsNull,
    IsNotNull,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_jsonb_index_selection() {
        let stats = ColumnStats {
            schema_name: "public".to_string(),
            table_name: "orders".to_string(),
            column_name: "metadata".to_string(),
            data_type: PostgresType::Jsonb,
            has_json_ops: true,
            total_rows: 1_000_000,
            cardinality_ratio: 0.95,
            distinct_values: 950_000,
            null_fraction: 0.0,
            correlation: 0.0,
            avg_width: 100,
            most_common_values: vec![],
            most_common_freqs: vec![],
            has_array_ops: false,
            has_text_patterns: false,
            has_geometric_ops: false,
            has_range_queries: false,
            is_sequential: false,
        };

        let selector = IndexTypeSelector::new();
        let rec = selector.select_index_type(&stats, &[]);

        assert_eq!(rec.index_type, IndexType::GIN);
        assert_eq!(rec.opclass, Some("jsonb_path_ops".to_string()));
        assert!(rec.confidence_score > 0.9);
    }

    #[test]
    fn test_brin_for_sequential_data() {
        let stats = ColumnStats {
            schema_name: "public".to_string(),
            table_name: "events".to_string(),
            column_name: "created_at".to_string(),
            data_type: PostgresType::Timestamp,
            total_rows: 10_000_000,
            correlation: 0.95,
            is_sequential: true,
            has_json_ops: false,
            cardinality_ratio: 0.8,
            distinct_values: 8_000_000,
            null_fraction: 0.0,
            avg_width: 8,
            most_common_values: vec![],
            most_common_freqs: vec![],
            has_array_ops: false,
            has_text_patterns: false,
            has_geometric_ops: false,
            has_range_queries: true,
        };

        let selector = IndexTypeSelector::new();
        let rec = selector.select_index_type(&stats, &[]);

        assert_eq!(rec.index_type, IndexType::BRIN);
        assert!(rec.estimated_size_mb < 5.0); // BRIN is tiny (< 5MB for 10M rows)
    }

    #[test]
    fn test_partial_index_for_skewed_data() {
        let stats = ColumnStats {
            schema_name: "public".to_string(),
            table_name: "orders".to_string(),
            column_name: "status".to_string(),
            data_type: PostgresType::Varchar,
            total_rows: 1_000_000,
            cardinality_ratio: 0.005, // Low cardinality
            distinct_values: 5_000,
            null_fraction: 0.0,
            correlation: 0.1,
            avg_width: 10,
            most_common_values: vec!["completed".to_string()],
            most_common_freqs: vec![0.95], // 95% completed
            has_json_ops: false,
            has_array_ops: false,
            has_text_patterns: false,
            has_geometric_ops: false,
            has_range_queries: false,
            is_sequential: false,
        };

        let selector = IndexTypeSelector::new();
        let rec = selector.select_index_type(&stats, &[]);

        assert_eq!(rec.index_type, IndexType::BTree);
        assert!(rec.where_clause.is_some());
        assert!(rec.where_clause.unwrap().contains("!= 'completed'"));
    }
}
