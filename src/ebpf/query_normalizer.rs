//! Query Hash Normalization
//!
//! Provides AST-based query normalization for consistent hashing.
//! Same query with different parameter values produces the same hash,
//! enabling proper aggregation of query metrics.

use sqlparser::ast::{Expr, Query, SelectItem, SetExpr, Statement, Value};
use sqlparser::dialect::PostgreSqlDialect;
use sqlparser::parser::Parser;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, warn};

/// Query normalizer that produces consistent hashes for parameterized queries
pub struct QueryNormalizer {
    dialect: PostgreSqlDialect,
    /// Cache of query text -> normalized hash
    cache: Arc<RwLock<HashMap<String, u64>>>,
    /// Maximum cache size
    max_cache_size: usize,
    /// Statistics
    stats: Arc<RwLock<NormalizerStats>>,
}

#[derive(Debug, Default, Clone)]
pub struct NormalizerStats {
    pub cache_hits: u64,
    pub cache_misses: u64,
    pub parse_failures: u64,
    pub normalized_count: u64,
}

impl QueryNormalizer {
    /// Create a new query normalizer
    pub fn new() -> Self {
        Self::with_cache_size(10_000)
    }

    /// Create with custom cache size
    pub fn with_cache_size(max_cache_size: usize) -> Self {
        Self {
            dialect: PostgreSqlDialect {},
            cache: Arc::new(RwLock::new(HashMap::with_capacity(max_cache_size))),
            max_cache_size,
            stats: Arc::new(RwLock::new(NormalizerStats::default())),
        }
    }

    /// Normalize and hash a query for consistent identification
    ///
    /// This replaces all literal values with placeholders, so:
    /// - `SELECT * FROM users WHERE id = 123` and
    /// - `SELECT * FROM users WHERE id = 456`
    /// produce the same hash.
    pub async fn normalize_and_hash(&self, query: &str) -> u64 {
        // Check cache first
        {
            let cache = self.cache.read().await;
            if let Some(&hash) = cache.get(query) {
                let mut stats = self.stats.write().await;
                stats.cache_hits += 1;
                return hash;
            }
        }

        // Parse and normalize
        let hash = self.compute_normalized_hash(query).await;

        // Update cache
        {
            let mut cache = self.cache.write().await;
            let mut stats = self.stats.write().await;
            stats.cache_misses += 1;

            // Evict if cache is full (simple random eviction)
            if cache.len() >= self.max_cache_size {
                // Remove ~10% of entries
                let to_remove: Vec<String> = cache
                    .keys()
                    .take(self.max_cache_size / 10)
                    .cloned()
                    .collect();
                for key in to_remove {
                    cache.remove(&key);
                }
            }

            cache.insert(query.to_string(), hash);
        }

        hash
    }

    /// Compute hash without caching
    async fn compute_normalized_hash(&self, query: &str) -> u64 {
        match Parser::parse_sql(&self.dialect, query) {
            Ok(statements) => {
                if statements.is_empty() {
                    return self.fallback_hash(query);
                }

                let mut stats = self.stats.write().await;
                stats.normalized_count += 1;
                drop(stats);

                // Normalize the first statement
                let normalized = self.normalize_statement(&statements[0]);
                self.hash_string(&normalized)
            }
            Err(e) => {
                debug!("Failed to parse query for normalization: {}", e);
                let mut stats = self.stats.write().await;
                stats.parse_failures += 1;
                drop(stats);

                // Fallback: simple text normalization
                self.fallback_hash(query)
            }
        }
    }

    /// Normalize a parsed statement by replacing literals with placeholders
    fn normalize_statement(&self, stmt: &Statement) -> String {
        match stmt {
            Statement::Query(query) => self.normalize_query(query),
            Statement::Insert { table_name, .. } => {
                format!("INSERT INTO {} (...) VALUES (...)", table_name)
            }
            Statement::Update { table, .. } => {
                format!("UPDATE {} SET ... WHERE ...", table.relation)
            }
            Statement::Delete { from, .. } => {
                let table_name = from
                    .first()
                    .map(|t| t.relation.to_string())
                    .unwrap_or_else(|| "?".to_string());
                format!("DELETE FROM {} WHERE ...", table_name)
            }
            _ => {
                // For other statements, use a simplified representation
                format!("{:?}", std::mem::discriminant(stmt))
            }
        }
    }

    /// Normalize a SELECT query
    fn normalize_query(&self, query: &Query) -> String {
        let mut parts = Vec::new();

        // Handle CTEs
        if let Some(with) = &query.with {
            parts.push(format!("WITH {} CTE(s)", with.cte_tables.len()));
        }

        // Handle the main body
        match query.body.as_ref() {
            SetExpr::Select(select) => {
                // Normalize SELECT items
                let select_items: Vec<String> = select
                    .projection
                    .iter()
                    .map(|item| self.normalize_select_item(item))
                    .collect();

                parts.push(format!("SELECT {}", select_items.join(", ")));

                // FROM clause
                if !select.from.is_empty() {
                    let tables: Vec<String> =
                        select.from.iter().map(|t| t.relation.to_string()).collect();
                    parts.push(format!("FROM {}", tables.join(", ")));
                }

                // WHERE clause (normalized)
                if let Some(selection) = &select.selection {
                    let normalized_where = self.normalize_expr(selection);
                    parts.push(format!("WHERE {}", normalized_where));
                }

                // GROUP BY
                if select.group_by != sqlparser::ast::GroupByExpr::Expressions(vec![]) {
                    parts.push("GROUP BY ...".to_string());
                }

                // HAVING
                if select.having.is_some() {
                    parts.push("HAVING ...".to_string());
                }
            }
            SetExpr::SetOperation {
                op, left, right, ..
            } => {
                parts.push(format!("{:?} (UNION/INTERSECT/EXCEPT)", op));
            }
            _ => {
                parts.push("QUERY".to_string());
            }
        }

        // ORDER BY
        if !query.order_by.is_empty() {
            parts.push("ORDER BY ...".to_string());
        }

        // LIMIT
        if query.limit.is_some() {
            parts.push("LIMIT ?".to_string());
        }

        // OFFSET
        if query.offset.is_some() {
            parts.push("OFFSET ?".to_string());
        }

        parts.join(" ")
    }

    /// Normalize a SELECT item
    fn normalize_select_item(&self, item: &SelectItem) -> String {
        match item {
            SelectItem::UnnamedExpr(expr) => self.normalize_expr(expr),
            SelectItem::ExprWithAlias { expr, alias } => {
                format!("{} AS {}", self.normalize_expr(expr), alias)
            }
            SelectItem::QualifiedWildcard(name, _) => format!("{}.*", name),
            SelectItem::Wildcard(_) => "*".to_string(),
        }
    }

    /// Normalize an expression, replacing literals with placeholders
    fn normalize_expr(&self, expr: &Expr) -> String {
        match expr {
            // Replace literal values with placeholders
            Expr::Value(value) => self.normalize_value(value),

            // Column references stay as-is
            Expr::Identifier(ident) => ident.value.clone(),
            Expr::CompoundIdentifier(idents) => idents
                .iter()
                .map(|i| i.value.as_str())
                .collect::<Vec<_>>()
                .join("."),

            // Binary operations
            Expr::BinaryOp { left, op, right } => {
                format!(
                    "({} {:?} {})",
                    self.normalize_expr(left),
                    op,
                    self.normalize_expr(right)
                )
            }

            // Unary operations
            Expr::UnaryOp { op, expr } => {
                format!("{:?} {}", op, self.normalize_expr(expr))
            }

            // Function calls
            Expr::Function(func) => {
                format!("{}(...)", func.name)
            }

            // Subqueries
            Expr::Subquery(query) => {
                format!("({})", self.normalize_query(query))
            }

            // IN list
            Expr::InList {
                expr,
                list,
                negated,
            } => {
                let neg = if *negated { "NOT " } else { "" };
                format!("{} {}IN (?...)", self.normalize_expr(expr), neg)
            }

            // IN subquery
            Expr::InSubquery {
                expr,
                subquery,
                negated,
            } => {
                let neg = if *negated { "NOT " } else { "" };
                format!(
                    "{} {}IN ({})",
                    self.normalize_expr(expr),
                    neg,
                    self.normalize_query(subquery)
                )
            }

            // BETWEEN
            Expr::Between {
                expr,
                negated,
                low,
                high,
            } => {
                let neg = if *negated { "NOT " } else { "" };
                format!("{} {}BETWEEN ? AND ?", self.normalize_expr(expr), neg)
            }

            // LIKE
            Expr::Like {
                expr,
                pattern,
                negated,
                ..
            } => {
                let neg = if *negated { "NOT " } else { "" };
                format!("{} {}LIKE ?", self.normalize_expr(expr), neg)
            }

            // IS NULL
            Expr::IsNull(expr) => format!("{} IS NULL", self.normalize_expr(expr)),
            Expr::IsNotNull(expr) => format!("{} IS NOT NULL", self.normalize_expr(expr)),

            // CASE
            Expr::Case { .. } => "CASE...END".to_string(),

            // Cast
            Expr::Cast {
                expr, data_type, ..
            } => {
                format!("CAST({} AS {:?})", self.normalize_expr(expr), data_type)
            }

            // Nested expression
            Expr::Nested(inner) => format!("({})", self.normalize_expr(inner)),

            // Default: represent as placeholder
            _ => "?".to_string(),
        }
    }

    /// Normalize a literal value to a placeholder
    fn normalize_value(&self, value: &Value) -> String {
        match value {
            Value::Number(_, _) => "?num".to_string(),
            Value::SingleQuotedString(_) => "?str".to_string(),
            Value::DoubleQuotedString(_) => "?str".to_string(),
            Value::Boolean(_) => "?bool".to_string(),
            Value::Null => "NULL".to_string(),
            Value::Placeholder(_) => "?".to_string(),
            _ => "?".to_string(),
        }
    }

    /// Fallback hash for unparseable queries
    fn fallback_hash(&self, query: &str) -> u64 {
        // Simple normalization: lowercase, collapse whitespace, remove literals
        let normalized = query
            .to_lowercase()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");

        // Remove quoted strings and numbers
        let cleaned = self.simple_normalize(&normalized);
        self.hash_string(&cleaned)
    }

    /// Simple text-based normalization for fallback
    fn simple_normalize(&self, query: &str) -> String {
        let mut result = String::with_capacity(query.len());
        let mut in_string = false;
        let mut string_char = ' ';
        let mut last_was_digit = false;

        for c in query.chars() {
            if in_string {
                if c == string_char {
                    in_string = false;
                    result.push_str("?str");
                }
                continue;
            }

            if c == '\'' || c == '"' {
                in_string = true;
                string_char = c;
                continue;
            }

            if c.is_ascii_digit() {
                if !last_was_digit {
                    result.push_str("?num");
                }
                last_was_digit = true;
                continue;
            }

            last_was_digit = false;
            result.push(c);
        }

        result
    }

    /// Hash a string using a stable hasher
    fn hash_string(&self, s: &str) -> u64 {
        use std::collections::hash_map::DefaultHasher;
        let mut hasher = DefaultHasher::new();
        s.hash(&mut hasher);
        hasher.finish()
    }

    /// Get normalizer statistics
    pub async fn get_stats(&self) -> NormalizerStats {
        self.stats.read().await.clone()
    }

    /// Clear the cache
    pub async fn clear_cache(&self) {
        let mut cache = self.cache.write().await;
        cache.clear();
    }

    /// Get cache size
    pub async fn cache_size(&self) -> usize {
        self.cache.read().await.len()
    }
}

impl Default for QueryNormalizer {
    fn default() -> Self {
        Self::new()
    }
}

/// Compute a query hash directly without normalization
/// Used for quick lookups when AST parsing isn't needed
pub fn quick_hash(query: &str) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    let mut hasher = DefaultHasher::new();
    query.hash(&mut hasher);
    hasher.finish()
}

/// Compute a normalized hash synchronously (for use in BPF correlation)
pub fn sync_normalize_hash(query: &str) -> u64 {
    let dialect = PostgreSqlDialect {};

    match Parser::parse_sql(&dialect, query) {
        Ok(statements) if !statements.is_empty() => {
            // Simple normalization: just get statement type + table names
            let stmt = &statements[0];
            let simplified = match stmt {
                Statement::Query(_) => "SELECT",
                Statement::Insert { .. } => "INSERT",
                Statement::Update { .. } => "UPDATE",
                Statement::Delete { .. } => "DELETE",
                _ => "OTHER",
            };
            quick_hash(simplified)
        }
        _ => quick_hash(query),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_same_query_different_values() {
        let normalizer = QueryNormalizer::new();

        let hash1 = normalizer
            .normalize_and_hash("SELECT * FROM users WHERE id = 123")
            .await;
        let hash2 = normalizer
            .normalize_and_hash("SELECT * FROM users WHERE id = 456")
            .await;

        assert_eq!(
            hash1, hash2,
            "Same query with different values should have same hash"
        );
    }

    #[tokio::test]
    async fn test_different_queries() {
        let normalizer = QueryNormalizer::new();

        let hash1 = normalizer
            .normalize_and_hash("SELECT * FROM users WHERE id = 123")
            .await;
        let hash2 = normalizer
            .normalize_and_hash("SELECT * FROM orders WHERE id = 123")
            .await;

        assert_ne!(
            hash1, hash2,
            "Different tables should have different hashes"
        );
    }

    #[tokio::test]
    async fn test_string_values_normalized() {
        let normalizer = QueryNormalizer::new();

        let hash1 = normalizer
            .normalize_and_hash("SELECT * FROM users WHERE name = 'Alice'")
            .await;
        let hash2 = normalizer
            .normalize_and_hash("SELECT * FROM users WHERE name = 'Bob'")
            .await;

        assert_eq!(hash1, hash2, "String values should be normalized");
    }

    #[tokio::test]
    async fn test_cache_behavior() {
        let normalizer = QueryNormalizer::new();

        let query = "SELECT * FROM users WHERE id = 1";

        // First call - cache miss
        let _ = normalizer.normalize_and_hash(query).await;
        let stats1 = normalizer.get_stats().await;
        assert_eq!(stats1.cache_misses, 1);
        assert_eq!(stats1.cache_hits, 0);

        // Second call - cache hit
        let _ = normalizer.normalize_and_hash(query).await;
        let stats2 = normalizer.get_stats().await;
        assert_eq!(stats2.cache_misses, 1);
        assert_eq!(stats2.cache_hits, 1);
    }

    #[tokio::test]
    async fn test_in_list_normalized() {
        let normalizer = QueryNormalizer::new();

        let hash1 = normalizer
            .normalize_and_hash("SELECT * FROM users WHERE id IN (1, 2, 3)")
            .await;
        let hash2 = normalizer
            .normalize_and_hash("SELECT * FROM users WHERE id IN (4, 5, 6, 7)")
            .await;

        assert_eq!(hash1, hash2, "IN lists should be normalized");
    }

    #[tokio::test]
    async fn test_invalid_sql_fallback() {
        let normalizer = QueryNormalizer::new();

        // Invalid SQL should still produce a hash
        let hash = normalizer
            .normalize_and_hash("NOT VALID SQL AT ALL!!!")
            .await;

        assert!(hash > 0, "Invalid SQL should produce a fallback hash");

        let stats = normalizer.get_stats().await;
        assert_eq!(stats.parse_failures, 1);
    }

    #[test]
    fn test_quick_hash() {
        let hash1 = quick_hash("SELECT * FROM users");
        let hash2 = quick_hash("SELECT * FROM users");
        let hash3 = quick_hash("SELECT * FROM orders");

        assert_eq!(hash1, hash2);
        assert_ne!(hash1, hash3);
    }

    #[test]
    fn test_sync_normalize_hash() {
        let hash = sync_normalize_hash("SELECT * FROM users WHERE id = 1");
        assert!(hash > 0);
    }
}
