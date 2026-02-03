//! Recommendation explainer - generates deterministic "why this recommendation" explanations.
//!
//! This module provides rule-based explanations for index recommendations based on evidence:
//! - Triggering query pattern and table/columns
//! - Expected mechanism (avoid seq scan, etc.)
//! - EXPLAIN proof summary
//! - Tradeoff estimates (write overhead, storage cost)
//!
//! NOTE: This module does NOT use LLM text generation. All explanations are rule-based.

use crate::analyzer::validation::{ValidationResult, ValidationStatus};
use serde::{Deserialize, Serialize};

/// A structured explanation for why an index is recommended
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecommendationExplanation {
    /// One-line summary
    pub summary: String,

    /// The triggering query pattern
    pub triggering_query: QueryPatternSummary,

    /// Expected mechanism improvement
    pub expected_improvement: ExpectedImprovement,

    /// EXPLAIN-based proof (if available)
    pub explain_proof: Option<ExplainProofSummary>,

    /// Estimated tradeoffs
    pub tradeoffs: TradeoffEstimate,

    /// Confidence level (high, medium, low)
    pub confidence: ConfidenceLevel,

    /// Key reasons supporting this recommendation
    pub reasons: Vec<String>,
}

/// Summary of the query pattern that triggered the recommendation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryPatternSummary {
    /// Table being queried
    pub table_name: String,

    /// Columns in the WHERE clause
    pub filter_columns: Vec<String>,

    /// Columns recommended for indexing
    pub index_columns: Vec<String>,

    /// Query pattern type (equality, range, join, etc.)
    pub pattern_type: QueryPatternType,

    /// Estimated query frequency (queries/hour)
    pub estimated_frequency: Option<f64>,

    /// Total execution time observed (seconds)
    pub total_exec_time_secs: Option<f64>,
}

/// Type of query pattern
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum QueryPatternType {
    /// Equality filter (col = value)
    Equality,
    /// Range filter (col > value, col BETWEEN, etc.)
    Range,
    /// IN list (col IN (...))
    InList,
    /// LIKE pattern (col LIKE 'prefix%')
    PrefixLike,
    /// Join condition
    Join,
    /// ORDER BY optimization
    OrderBy,
    /// GROUP BY optimization
    GroupBy,
    /// Combined (multiple patterns)
    Combined,
    /// Unknown pattern
    Unknown,
}

impl QueryPatternType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Equality => "equality filter",
            Self::Range => "range filter",
            Self::InList => "IN list filter",
            Self::PrefixLike => "prefix LIKE pattern",
            Self::Join => "join condition",
            Self::OrderBy => "ORDER BY optimization",
            Self::GroupBy => "GROUP BY optimization",
            Self::Combined => "combined filter patterns",
            Self::Unknown => "filter pattern",
        }
    }
}

/// Expected improvement from the index
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExpectedImprovement {
    /// Current scan method being used
    pub current_method: String,

    /// Expected scan method with index
    pub expected_method: String,

    /// Human-readable explanation
    pub explanation: String,

    /// Expected query time reduction (percentage)
    pub estimated_speedup_pct: Option<f64>,
}

/// Summary of EXPLAIN-based proof
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExplainProofSummary {
    /// Whether validation passed
    pub validated: bool,

    /// Plan change description
    pub plan_change: String,

    /// Cost improvement percentage
    pub cost_improvement_pct: f64,

    /// Whether the index was used by the planner
    pub index_used: bool,

    /// Before scan type
    pub before_scan: Option<String>,

    /// After scan type
    pub after_scan: Option<String>,
}

/// Estimated tradeoffs of creating the index
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradeoffEstimate {
    /// Estimated storage size (human readable)
    pub estimated_storage: String,

    /// Estimated write overhead description
    pub write_overhead: String,

    /// Whether this is a high-write table
    pub high_write_table: bool,

    /// Index maintenance warning (if any)
    pub maintenance_warning: Option<String>,

    /// Overall tradeoff assessment
    pub assessment: String,
}

/// Confidence level for the recommendation
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConfidenceLevel {
    /// High confidence - EXPLAIN proof shows significant improvement
    High,
    /// Medium confidence - heuristics suggest improvement
    Medium,
    /// Low confidence - may or may not help
    Low,
}

impl ConfidenceLevel {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::High => "HIGH",
            Self::Medium => "MEDIUM",
            Self::Low => "LOW",
        }
    }
}

/// Generates recommendation explanations
pub struct RecommendationExplainer;

impl RecommendationExplainer {
    /// Generate an explanation for an index recommendation
    pub fn explain(
        table_name: &str,
        columns: &[String],
        query_pattern: QueryPatternType,
        validation_result: Option<&ValidationResult>,
        table_stats: Option<&TableStats>,
    ) -> RecommendationExplanation {
        let mut reasons = Vec::new();

        // Build query pattern summary
        let triggering_query = QueryPatternSummary {
            table_name: table_name.to_string(),
            filter_columns: columns.to_vec(),
            index_columns: columns.to_vec(),
            pattern_type: query_pattern,
            estimated_frequency: table_stats.and_then(|s| s.queries_per_hour),
            total_exec_time_secs: table_stats.and_then(|s| s.total_exec_time_secs),
        };

        // Determine expected improvement based on pattern
        let expected_improvement = Self::determine_expected_improvement(query_pattern, columns);
        reasons.push(expected_improvement.explanation.clone());

        // Build EXPLAIN proof summary if available
        let explain_proof = validation_result.and_then(|vr| {
            vr.diff_summary.as_ref().map(|diff| {
                ExplainProofSummary {
                    validated: vr.status == ValidationStatus::Validated,
                    plan_change: diff.summary.clone(),
                    cost_improvement_pct: diff.cost_improvement_pct,
                    index_used: diff.index_used,
                    before_scan: diff.before_scan_type.map(|s| s.as_str().to_string()),
                    after_scan: diff.after_scan_type.map(|s| s.as_str().to_string()),
                }
            })
        });

        // Add EXPLAIN proof to reasons if available
        if let Some(ref proof) = explain_proof {
            if proof.validated {
                reasons.push(format!(
                    "EXPLAIN proof shows {:.1}% cost reduction",
                    proof.cost_improvement_pct
                ));
                if proof.index_used {
                    reasons.push("Query planner confirms index will be used".to_string());
                }
            }
        }

        // Calculate tradeoffs
        let tradeoffs = Self::estimate_tradeoffs(table_name, columns, table_stats);

        // Determine confidence level
        let confidence = Self::determine_confidence(&explain_proof, table_stats);

        // Generate summary
        let summary = Self::generate_summary(
            table_name,
            columns,
            query_pattern,
            &explain_proof,
            confidence,
        );

        RecommendationExplanation {
            summary,
            triggering_query,
            expected_improvement,
            explain_proof,
            tradeoffs,
            confidence,
            reasons,
        }
    }

    fn determine_expected_improvement(
        pattern: QueryPatternType,
        columns: &[String],
    ) -> ExpectedImprovement {
        let col_str = columns.join(", ");

        match pattern {
            QueryPatternType::Equality => ExpectedImprovement {
                current_method: "Sequential Scan".to_string(),
                expected_method: "Index Scan".to_string(),
                explanation: format!(
                    "Equality filter on {} can use B-tree index for O(log n) lookup instead of O(n) scan",
                    col_str
                ),
                estimated_speedup_pct: Some(90.0), // Typical for selective equality
            },
            QueryPatternType::Range => ExpectedImprovement {
                current_method: "Sequential Scan".to_string(),
                expected_method: "Index Scan or Bitmap Index Scan".to_string(),
                explanation: format!(
                    "Range filter on {} can use B-tree index to limit scan to matching range",
                    col_str
                ),
                estimated_speedup_pct: Some(60.0), // Varies by selectivity
            },
            QueryPatternType::InList => ExpectedImprovement {
                current_method: "Sequential Scan".to_string(),
                expected_method: "Bitmap Index Scan".to_string(),
                explanation: format!(
                    "IN list on {} can use index bitmap to efficiently combine multiple lookups",
                    col_str
                ),
                estimated_speedup_pct: Some(70.0),
            },
            QueryPatternType::PrefixLike => ExpectedImprovement {
                current_method: "Sequential Scan with Filter".to_string(),
                expected_method: "Index Scan".to_string(),
                explanation: format!(
                    "Prefix LIKE pattern on {} can use B-tree index for range scan",
                    col_str
                ),
                estimated_speedup_pct: Some(80.0),
            },
            QueryPatternType::Join => ExpectedImprovement {
                current_method: "Nested Loop or Hash Join".to_string(),
                expected_method: "Index Nested Loop Join".to_string(),
                explanation: format!(
                    "Join on {} can use index to speed up inner loop lookups",
                    col_str
                ),
                estimated_speedup_pct: Some(75.0),
            },
            QueryPatternType::OrderBy => ExpectedImprovement {
                current_method: "Sequential Scan + Sort".to_string(),
                expected_method: "Index Scan (ordered)".to_string(),
                explanation: format!(
                    "ORDER BY {} can use index to avoid sorting",
                    col_str
                ),
                estimated_speedup_pct: Some(50.0),
            },
            QueryPatternType::GroupBy => ExpectedImprovement {
                current_method: "Sequential Scan + Hash/Sort Aggregate".to_string(),
                expected_method: "Index Scan + Streaming Aggregate".to_string(),
                explanation: format!(
                    "GROUP BY {} can use index for efficient aggregation",
                    col_str
                ),
                estimated_speedup_pct: Some(40.0),
            },
            QueryPatternType::Combined | QueryPatternType::Unknown => ExpectedImprovement {
                current_method: "Sequential Scan".to_string(),
                expected_method: "Index Scan".to_string(),
                explanation: format!(
                    "Index on {} expected to reduce query time by avoiding full table scan",
                    col_str
                ),
                estimated_speedup_pct: None,
            },
        }
    }

    fn estimate_tradeoffs(
        _table_name: &str,
        columns: &[String],
        table_stats: Option<&TableStats>,
    ) -> TradeoffEstimate {
        let (estimated_storage, write_overhead, high_write, maintenance_warning) =
            if let Some(stats) = table_stats {
                let row_count = stats.row_count.unwrap_or(10000);
                let avg_row_width = stats.avg_row_width.unwrap_or(100);

                // Rough estimate: index is ~20-30% of table size for single column
                let column_count = columns.len();
                let index_ratio = 0.2 + (column_count as f64 * 0.1);
                let table_size_bytes = row_count as f64 * avg_row_width as f64;
                let index_size_bytes = table_size_bytes * index_ratio;

                let storage = Self::format_bytes(index_size_bytes as u64);

                // Determine write overhead
                let writes_per_sec = stats.writes_per_second.unwrap_or(0.0);
                let high_write = writes_per_sec > 100.0;

                let write_overhead = if high_write {
                    format!(
                        "Moderate: ~{:.0} writes/sec will have ~10-20% overhead per write",
                        writes_per_sec
                    )
                } else if writes_per_sec > 10.0 {
                    "Low: Write rate is moderate, index maintenance overhead is acceptable".to_string()
                } else {
                    "Minimal: Low write rate means negligible index maintenance overhead".to_string()
                };

                let warning = if high_write && columns.len() > 2 {
                    Some(format!(
                        "High-write table with multi-column index may have noticeable write overhead"
                    ))
                } else {
                    None
                };

                (storage, write_overhead, high_write, warning)
            } else {
                (
                    "Unknown (table stats not available)".to_string(),
                    "Unknown write overhead".to_string(),
                    false,
                    None,
                )
            };

        let assessment = if high_write {
            "Monitor write performance after index creation".to_string()
        } else {
            "Write overhead is expected to be acceptable".to_string()
        };

        TradeoffEstimate {
            estimated_storage,
            write_overhead,
            high_write_table: high_write,
            maintenance_warning,
            assessment,
        }
    }

    fn determine_confidence(
        explain_proof: &Option<ExplainProofSummary>,
        table_stats: Option<&TableStats>,
    ) -> ConfidenceLevel {
        if let Some(proof) = explain_proof {
            if proof.validated && proof.index_used && proof.cost_improvement_pct > 50.0 {
                return ConfidenceLevel::High;
            } else if proof.validated && proof.index_used {
                return ConfidenceLevel::Medium;
            }
        }

        // Without EXPLAIN proof, use heuristics
        if let Some(stats) = table_stats {
            if stats.row_count.unwrap_or(0) > 10000 && stats.total_exec_time_secs.unwrap_or(0.0) > 1.0
            {
                return ConfidenceLevel::Medium;
            }
        }

        ConfidenceLevel::Low
    }

    fn generate_summary(
        table_name: &str,
        columns: &[String],
        pattern: QueryPatternType,
        explain_proof: &Option<ExplainProofSummary>,
        confidence: ConfidenceLevel,
    ) -> String {
        let col_str = if columns.len() == 1 {
            columns[0].clone()
        } else {
            format!("({})", columns.join(", "))
        };

        if let Some(proof) = explain_proof {
            if proof.validated {
                format!(
                    "Add index on {}.{} for {} — EXPLAIN proof shows {:.0}% cost reduction [{}]",
                    table_name,
                    col_str,
                    pattern.as_str(),
                    proof.cost_improvement_pct,
                    confidence.as_str()
                )
            } else {
                format!(
                    "Index on {}.{} for {} — validation did not show improvement [{}]",
                    table_name,
                    col_str,
                    pattern.as_str(),
                    confidence.as_str()
                )
            }
        } else {
            format!(
                "Add index on {}.{} to optimize {} queries [{}]",
                table_name,
                col_str,
                pattern.as_str(),
                confidence.as_str()
            )
        }
    }

    fn format_bytes(bytes: u64) -> String {
        const KB: u64 = 1024;
        const MB: u64 = 1024 * KB;
        const GB: u64 = 1024 * MB;

        if bytes >= GB {
            format!("{:.1} GB", bytes as f64 / GB as f64)
        } else if bytes >= MB {
            format!("{:.1} MB", bytes as f64 / MB as f64)
        } else if bytes >= KB {
            format!("{:.1} KB", bytes as f64 / KB as f64)
        } else {
            format!("{} bytes", bytes)
        }
    }
}

/// Table statistics for tradeoff estimation
#[derive(Debug, Clone, Default)]
pub struct TableStats {
    pub row_count: Option<i64>,
    pub avg_row_width: Option<i32>,
    pub writes_per_second: Option<f64>,
    pub queries_per_hour: Option<f64>,
    pub total_exec_time_secs: Option<f64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_query_pattern_type_as_str() {
        assert_eq!(QueryPatternType::Equality.as_str(), "equality filter");
        assert_eq!(QueryPatternType::Range.as_str(), "range filter");
        assert_eq!(QueryPatternType::Join.as_str(), "join condition");
    }

    #[test]
    fn test_confidence_level_as_str() {
        assert_eq!(ConfidenceLevel::High.as_str(), "HIGH");
        assert_eq!(ConfidenceLevel::Medium.as_str(), "MEDIUM");
        assert_eq!(ConfidenceLevel::Low.as_str(), "LOW");
    }

    #[test]
    fn test_explain_without_proof() {
        let explanation = RecommendationExplainer::explain(
            "users",
            &["email".to_string()],
            QueryPatternType::Equality,
            None,
            None,
        );

        assert!(explanation.summary.contains("users"));
        assert!(explanation.summary.contains("email"));
        assert_eq!(explanation.triggering_query.table_name, "users");
        assert_eq!(explanation.confidence, ConfidenceLevel::Low);
        assert!(explanation.explain_proof.is_none());
    }

    #[test]
    fn test_explain_with_table_stats() {
        let stats = TableStats {
            row_count: Some(1_000_000),
            avg_row_width: Some(200),
            writes_per_second: Some(5.0),
            queries_per_hour: Some(1000.0),
            total_exec_time_secs: Some(60.0),
        };

        let explanation = RecommendationExplainer::explain(
            "orders",
            &["customer_id".to_string()],
            QueryPatternType::Equality,
            None,
            Some(&stats),
        );

        assert!(explanation.tradeoffs.estimated_storage.contains("MB") || 
                explanation.tradeoffs.estimated_storage.contains("GB"));
        assert!(!explanation.tradeoffs.high_write_table);
        assert_eq!(explanation.confidence, ConfidenceLevel::Medium);
    }

    #[test]
    fn test_explain_high_write_table() {
        let stats = TableStats {
            row_count: Some(100_000),
            avg_row_width: Some(100),
            writes_per_second: Some(500.0),
            queries_per_hour: None,
            total_exec_time_secs: None,
        };

        let explanation = RecommendationExplainer::explain(
            "events",
            &["user_id".to_string(), "created_at".to_string(), "event_type".to_string()],
            QueryPatternType::Combined,
            None,
            Some(&stats),
        );

        assert!(explanation.tradeoffs.high_write_table);
        assert!(explanation.tradeoffs.maintenance_warning.is_some());
    }

    #[test]
    fn test_format_bytes() {
        assert_eq!(RecommendationExplainer::format_bytes(500), "500 bytes");
        assert_eq!(RecommendationExplainer::format_bytes(2048), "2.0 KB");
        assert_eq!(RecommendationExplainer::format_bytes(5 * 1024 * 1024), "5.0 MB");
        assert_eq!(RecommendationExplainer::format_bytes(2 * 1024 * 1024 * 1024), "2.0 GB");
    }

    #[test]
    fn test_expected_improvement_equality() {
        let improvement = RecommendationExplainer::determine_expected_improvement(
            QueryPatternType::Equality,
            &["id".to_string()],
        );

        assert!(improvement.explanation.contains("O(log n)"));
        assert!(improvement.estimated_speedup_pct.is_some());
    }

    #[test]
    fn test_multi_column_index_explanation() {
        let explanation = RecommendationExplainer::explain(
            "orders",
            &["customer_id".to_string(), "order_date".to_string()],
            QueryPatternType::Combined,
            None,
            None,
        );

        assert!(explanation.summary.contains("customer_id"));
        assert!(explanation.summary.contains("order_date"));
    }
}
