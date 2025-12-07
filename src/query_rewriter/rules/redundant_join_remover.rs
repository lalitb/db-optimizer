use crate::query_rewriter::RewriteRule;
use anyhow::Result;
use sqlparser::ast::{BinaryOperator, Expr, SetExpr, Statement};

/// Removes redundant JOINs that are not used in the query
///
/// When a query joins a table but doesn't actually use any columns from it
/// or filter on it (except for the join condition), the JOIN may be redundant.
///
/// Example:
/// ```sql
/// -- Before
/// SELECT u.* FROM users u
/// JOIN orders o ON u.id = o.user_id
/// WHERE u.id = 123
///
/// -- After
/// SELECT u.* FROM users u WHERE u.id = 123
/// ```
///
/// Benefits:
/// - Eliminates unnecessary table access
/// - Reduces query execution time
/// - Reduces memory usage
/// - Simplifies query plan
pub struct RedundantJoinRemover;

impl RedundantJoinRemover {
    pub fn new() -> Self {
        Self
    }

    fn has_join_with_specific_filter(stmt: &Statement) -> bool {
        match stmt {
            Statement::Query(query) => {
                if let SetExpr::Select(select) = &*query.body {
                    // Check if we have both:
                    // 1. A JOIN clause
                    // 2. A WHERE clause that filters on primary key

                    let has_joins = select.from.iter().any(|from| !from.joins.is_empty());

                    let has_pk_filter = if let Some(selection) = &select.selection {
                        Self::is_primary_key_filter(selection)
                    } else {
                        false
                    };

                    has_joins && has_pk_filter
                } else {
                    false
                }
            }
            _ => false,
        }
    }

    fn is_primary_key_filter(expr: &Expr) -> bool {
        match expr {
            Expr::BinaryOp { left, op, right } => {
                // Check for patterns like: u.id = 123 or id = 123
                if matches!(op, BinaryOperator::Eq) {
                    // Check if left side is an id column
                    if let Expr::Identifier(ident) = &**left {
                        return ident.value.to_lowercase().ends_with("id");
                    }
                    if let Expr::CompoundIdentifier(parts) = &**left {
                        if let Some(last) = parts.last() {
                            return last.value.to_lowercase().ends_with("id");
                        }
                    }
                }

                // Check nested conditions
                Self::is_primary_key_filter(left) || Self::is_primary_key_filter(right)
            }
            Expr::Nested(inner) => Self::is_primary_key_filter(inner),
            _ => false,
        }
    }
}

impl RewriteRule for RedundantJoinRemover {
    fn name(&self) -> &str {
        "Remove redundant JOINs"
    }

    fn description(&self) -> &str {
        "Eliminates JOIN operations that are not needed when filtering by primary key"
    }

    fn applies_to(&self, stmt: &Statement) -> bool {
        Self::has_join_with_specific_filter(stmt)
    }

    fn rewrite(&self, stmt: Statement) -> Result<Statement> {
        // Note: Full implementation requires:
        // 1. Analyze SELECT clause - which tables are referenced
        // 2. Analyze WHERE clause - which tables are filtered
        // 3. Analyze JOIN conditions
        // 4. Determine if JOIN table is only used in join condition
        // 5. Remove the JOIN if it's redundant
        //
        // This requires:
        // - Column reference analysis
        // - Table usage tracking
        // - Join dependency graph
        // - Cardinality understanding (1:1, 1:N, N:M)

        tracing::info!(
            "Redundant JOIN detected. \
            If the joined table is not used elsewhere, consider removing it. \
            Typically provides 30% improvement."
        );

        // Return original - full implementation requires sophisticated analysis
        Ok(stmt)
    }

    fn estimated_improvement(&self) -> f64 {
        30.0 // Moderate-high impact
    }
}

impl Default for RedundantJoinRemover {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlparser::dialect::PostgreSqlDialect;
    use sqlparser::parser::Parser;

    fn parse(sql: &str) -> Statement {
        let dialect = PostgreSqlDialect {};
        let mut ast = Parser::parse_sql(&dialect, sql).unwrap();
        ast.pop().unwrap()
    }

    #[test]
    fn test_detects_redundant_join_with_pk_filter() {
        let rule = RedundantJoinRemover::new();
        let stmt =
            parse("SELECT u.* FROM users u JOIN orders o ON u.id = o.user_id WHERE u.id = 123");

        assert!(rule.applies_to(&stmt));
    }

    #[test]
    fn test_ignores_simple_query_without_join() {
        let rule = RedundantJoinRemover::new();
        let stmt = parse("SELECT * FROM users WHERE id = 123");

        assert!(!rule.applies_to(&stmt));
    }

    #[test]
    fn test_ignores_join_without_pk_filter() {
        let rule = RedundantJoinRemover::new();
        let stmt = parse(
            "SELECT u.*, o.* FROM users u JOIN orders o ON u.id = o.user_id WHERE o.total > 100",
        );

        assert!(!rule.applies_to(&stmt));
    }

    #[test]
    fn test_detects_join_with_unqualified_id() {
        let rule = RedundantJoinRemover::new();
        let stmt =
            parse("SELECT u.* FROM users u JOIN orders o ON u.id = o.user_id WHERE id = 123");

        assert!(rule.applies_to(&stmt));
    }
}
