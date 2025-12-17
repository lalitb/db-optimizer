use crate::query_rewriter::RewriteRule;
use anyhow::{Context, Result};
use sqlparser::ast::{Expr, Ident, SelectItem, SetExpr, Statement, TableFactor};
use sqlx::PgPool;

/// Enhanced SELECT * Remover with schema access
///
/// Replaces SELECT * with explicit column lists by querying the database schema.
/// This reduces data transfer and makes queries more maintainable.
pub struct SelectStarRemoverEnhanced {
    pool: Option<PgPool>,
}

impl SelectStarRemoverEnhanced {
    pub fn new() -> Self {
        Self { pool: None }
    }

    /// Create with database connection for schema queries
    pub fn with_pool(pool: PgPool) -> Self {
        Self { pool: Some(pool) }
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

    /// Get table name from FROM clause
    fn extract_table_name(stmt: &Statement) -> Option<String> {
        match stmt {
            Statement::Query(query) => {
                if let SetExpr::Select(select) = &*query.body {
                    if let Some(from) = select.from.first() {
                        match &from.relation {
                            TableFactor::Table { name, .. } => Some(name.to_string()),
                            _ => None,
                        }
                    } else {
                        None
                    }
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    /// Query database for column names
    async fn get_columns(&self, table_name: &str) -> Result<Vec<String>> {
        if let Some(pool) = &self.pool {
            // Parse schema.table if present
            let (schema, table) = if table_name.contains('.') {
                let parts: Vec<&str> = table_name.split('.').collect();
                (parts[0].trim_matches('"'), parts[1].trim_matches('"'))
            } else {
                ("public", table_name.trim_matches('"'))
            };

            let columns: Vec<(String,)> = sqlx::query_as(
                "SELECT column_name 
                 FROM information_schema.columns 
                 WHERE table_schema = $1 AND table_name = $2 
                 ORDER BY ordinal_position",
            )
            .bind(schema)
            .bind(table)
            .fetch_all(pool)
            .await
            .context("Failed to query column names")?;

            Ok(columns.into_iter().map(|(name,)| name).collect())
        } else {
            Err(anyhow::anyhow!("No database connection available"))
        }
    }

    /// Replace SELECT * with explicit columns (synchronous wrapper)
    fn replace_star_sync(&self, stmt: Statement, columns: Vec<String>) -> Result<Statement> {
        match stmt {
            Statement::Query(mut query) => {
                if let SetExpr::Select(ref mut select) = &mut *query.body {
                    // Replace wildcard with explicit columns
                    select.projection = columns
                        .into_iter()
                        .map(|col| SelectItem::UnnamedExpr(Expr::Identifier(Ident::new(col))))
                        .collect();
                }
                Ok(Statement::Query(query))
            }
            _ => Ok(stmt),
        }
    }
}

impl RewriteRule for SelectStarRemoverEnhanced {
    fn name(&self) -> &str {
        "Enhanced SELECT * Removal"
    }

    fn description(&self) -> &str {
        "Replaces SELECT * with explicit column list from schema"
    }

    fn applies_to(&self, stmt: &Statement) -> bool {
        Self::has_select_star(stmt) && self.pool.is_some()
    }

    fn rewrite(&self, stmt: Statement) -> Result<Statement> {
        if self.pool.is_none() {
            tracing::warn!(
                "SELECT * detected but no database connection available for schema query"
            );
            return Ok(stmt);
        }

        // Get table name
        let table_name = Self::extract_table_name(&stmt)
            .ok_or_else(|| anyhow::anyhow!("Could not extract table name"))?;

        // Query schema for columns (using blocking wrapper)
        let columns = tokio::runtime::Handle::try_current()
            .map(|handle| handle.block_on(async { self.get_columns(&table_name).await }))
            .unwrap_or_else(|_| {
                // No runtime available, use tokio runtime
                tokio::runtime::Runtime::new()
                    .unwrap()
                    .block_on(async { self.get_columns(&table_name).await })
            })?;

        if columns.is_empty() {
            return Err(anyhow::anyhow!("No columns found for table {}", table_name));
        }

        // Replace SELECT * with column list
        self.replace_star_sync(stmt, columns)
    }

    fn estimated_improvement(&self) -> f64 {
        15.0
    }
}

impl Default for SelectStarRemoverEnhanced {
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
        let _rule = SelectStarRemoverEnhanced::new();
        let stmt = parse("SELECT * FROM users");

        // Applies if we have a pool
        assert!(SelectStarRemoverEnhanced::has_select_star(&stmt));
    }

    #[test]
    fn test_extracts_table_name() {
        let stmt = parse("SELECT * FROM users");
        let table = SelectStarRemoverEnhanced::extract_table_name(&stmt);

        assert_eq!(table, Some("users".to_string()));
    }

    #[test]
    fn test_extracts_qualified_table_name() {
        let stmt = parse("SELECT * FROM public.users");
        let table = SelectStarRemoverEnhanced::extract_table_name(&stmt);

        assert_eq!(table, Some("public.users".to_string()));
    }

    #[test]
    fn test_without_pool_doesnt_apply() {
        let rule = SelectStarRemoverEnhanced::new();
        let stmt = parse("SELECT * FROM users");

        // Without pool, rule doesn't apply
        assert!(!rule.applies_to(&stmt));
    }
}
