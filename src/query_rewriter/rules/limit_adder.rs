use crate::query_rewriter::RewriteRule;
use anyhow::Result;
use sqlparser::ast::{Expr, Query, Select, SetExpr, Statement};

/// Adds LIMIT clause to unbounded queries that could return large result sets
///
/// This is one of the highest-impact optimizations as it prevents full table scans
/// when users accidentally write unbounded queries.
pub struct LimitAdder {
    default_limit: u64,
}

impl LimitAdder {
    pub fn new() -> Self {
        Self {
            default_limit: 1000,
        }
    }

    pub fn with_limit(mut self, limit: u64) -> Self {
        self.default_limit = limit;
        self
    }

    fn has_limit(query: &Query) -> bool {
        query.limit.is_some()
    }

    fn has_aggregation(select: &Select) -> bool {
        // Check for GROUP BY
        if !matches!(select.group_by, sqlparser::ast::GroupByExpr::Expressions(ref exprs) if exprs.is_empty())
        {
            return true;
        }

        // Check for aggregate functions in projection
        for item in &select.projection {
            if let sqlparser::ast::SelectItem::UnnamedExpr(expr)
            | sqlparser::ast::SelectItem::ExprWithAlias { expr, .. } = item
            {
                if Self::contains_aggregate(expr) {
                    return true;
                }
            }
        }

        false
    }

    fn contains_aggregate(expr: &Expr) -> bool {
        match expr {
            Expr::Function(func) => {
                let name = func.name.to_string().to_uppercase();
                matches!(
                    name.as_str(),
                    "COUNT"
                        | "SUM"
                        | "AVG"
                        | "MIN"
                        | "MAX"
                        | "STRING_AGG"
                        | "ARRAY_AGG"
                        | "JSON_AGG"
                )
            }
            _ => false,
        }
    }
}

impl RewriteRule for LimitAdder {
    fn name(&self) -> &str {
        "Add LIMIT to unbounded queries"
    }

    fn description(&self) -> &str {
        "Prevents full table scans by adding LIMIT clause to queries without one"
    }

    fn applies_to(&self, stmt: &Statement) -> bool {
        match stmt {
            Statement::Query(query) => {
                // Only apply if:
                // 1. No LIMIT clause exists
                // 2. Not an aggregation query
                if Self::has_limit(query) {
                    return false;
                }

                // Check if it's a simple SELECT
                if let SetExpr::Select(select) = &*query.body {
                    !Self::has_aggregation(select)
                } else {
                    false
                }
            }
            _ => false,
        }
    }

    fn rewrite(&self, stmt: Statement) -> Result<Statement> {
        match stmt {
            Statement::Query(mut query) => {
                // Add LIMIT clause
                query.limit = Some(Expr::Value(sqlparser::ast::Value::Number(
                    self.default_limit.to_string(),
                    false,
                )));

                Ok(Statement::Query(query))
            }
            _ => Ok(stmt),
        }
    }

    fn estimated_improvement(&self) -> f64 {
        90.0 // Very high impact - prevents full table scans
    }
}

impl Default for LimitAdder {
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
    fn test_adds_limit_to_unbounded_query() {
        let rule = LimitAdder::new();
        let stmt = parse("SELECT * FROM users");

        assert!(rule.applies_to(&stmt));

        let rewritten = rule.rewrite(stmt).unwrap();
        let sql = format!("{}", rewritten);

        assert!(sql.contains("LIMIT"));
        assert!(sql.contains("1000"));
    }

    #[test]
    fn test_skips_query_with_existing_limit() {
        let rule = LimitAdder::new();
        let stmt = parse("SELECT * FROM users LIMIT 10");

        assert!(!rule.applies_to(&stmt));
    }

    #[test]
    fn test_skips_aggregation_query() {
        let rule = LimitAdder::new();
        let stmt = parse("SELECT COUNT(*) FROM users");

        assert!(!rule.applies_to(&stmt));
    }

    #[test]
    fn test_skips_group_by_query() {
        let rule = LimitAdder::new();
        let stmt = parse("SELECT country, COUNT(*) FROM users GROUP BY country");

        assert!(!rule.applies_to(&stmt));
    }

    #[test]
    fn test_custom_limit() {
        let rule = LimitAdder::new().with_limit(500);
        let stmt = parse("SELECT * FROM users");

        let rewritten = rule.rewrite(stmt).unwrap();
        let sql = format!("{}", rewritten);

        assert!(sql.contains("500"));
    }
}
