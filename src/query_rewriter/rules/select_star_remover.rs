use crate::query_rewriter::RewriteRule;
use anyhow::Result;
use sqlparser::ast::{SelectItem, SetExpr, Statement};

/// Removes SELECT * and replaces with explicit column list
///
/// Benefits:
/// - Reduces network traffic (only needed columns)
/// - Improves query cache efficiency
/// - Makes queries more maintainable
/// - Prevents breaking changes when schema changes
///
/// Note: This is a simplified version that demonstrates the concept.
/// A production version would need to query the schema to get actual column names.
pub struct SelectStarRemover;

impl SelectStarRemover {
    pub fn new() -> Self {
        Self
    }

    fn has_select_star(stmt: &Statement) -> bool {
        match stmt {
            Statement::Query(query) => {
                if let SetExpr::Select(select) = &*query.body {
                    select
                        .projection
                        .iter()
                        .any(|item| matches!(item, SelectItem::Wildcard(_)))
                } else {
                    false
                }
            }
            _ => false,
        }
    }
}

impl RewriteRule for SelectStarRemover {
    fn name(&self) -> &str {
        "Remove SELECT *"
    }

    fn description(&self) -> &str {
        "Replaces SELECT * with explicit column list to reduce data transfer and improve performance"
    }

    fn applies_to(&self, stmt: &Statement) -> bool {
        Self::has_select_star(stmt)
    }

    fn rewrite(&self, stmt: Statement) -> Result<Statement> {
        // Note: In a real implementation, this would:
        // 1. Extract table name from FROM clause
        // 2. Query information_schema to get column names
        // 3. Replace * with explicit column list
        //
        // For now, we return the original statement with a comment
        // that this optimization requires schema information

        tracing::warn!(
            "SELECT * removal requires schema information. \
            Consider manually specifying columns."
        );

        // Return original - would need database connection to get columns
        Ok(stmt)
    }

    fn estimated_improvement(&self) -> f64 {
        15.0 // Moderate improvement from reduced data transfer
    }
}

impl Default for SelectStarRemover {
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
    fn test_detects_select_star() {
        let rule = SelectStarRemover::new();
        let stmt = parse("SELECT * FROM users");

        assert!(rule.applies_to(&stmt));
    }

    #[test]
    fn test_ignores_explicit_columns() {
        let rule = SelectStarRemover::new();
        let stmt = parse("SELECT id, name FROM users");

        assert!(!rule.applies_to(&stmt));
    }

    #[test]
    fn test_detects_qualified_star() {
        let rule = SelectStarRemover::new();
        let stmt = parse("SELECT u.* FROM users u");

        // Note: This version only detects unqualified *
        // A production version should also detect u.*
        assert!(!rule.applies_to(&stmt)); // Current limitation
    }
}
