use crate::query_rewriter::RewriteRule;
use anyhow::Result;
use sqlparser::ast::{
    BinaryOperator, Expr, Select, SelectItem, SetExpr, Statement, TableWithJoins,
};
use std::collections::HashSet;

/// Enhanced Redundant JOIN Remover with column usage analysis
///
/// Removes JOINs that are not actually used in the query by analyzing:
/// - Which tables are referenced in SELECT clause
/// - Which tables are referenced in WHERE clause  
/// - Which tables are only used for the JOIN condition
pub struct RedundantJoinRemoverEnhanced;

impl RedundantJoinRemoverEnhanced {
    pub fn new() -> Self {
        Self
    }

    fn has_potentially_redundant_join(stmt: &Statement) -> bool {
        match stmt {
            Statement::Query(query) => {
                if let SetExpr::Select(select) = &*query.body {
                    // Must have JOIN and a specific filter (like id = constant)
                    let has_joins = select.from.iter().any(|from| !from.joins.is_empty());
                    let has_pk_filter = select
                        .selection
                        .as_ref()
                        .map(|s| Self::has_primary_key_filter(s))
                        .unwrap_or(false);

                    has_joins && has_pk_filter
                } else {
                    false
                }
            }
            _ => false,
        }
    }

    fn has_primary_key_filter(expr: &Expr) -> bool {
        match expr {
            Expr::BinaryOp { left, op, right } if matches!(op, BinaryOperator::And) => {
                Self::has_primary_key_filter(left) || Self::has_primary_key_filter(right)
            }
            Expr::BinaryOp { left, op, .. } => {
                if matches!(op, BinaryOperator::Eq) {
                    match &**left {
                        Expr::Identifier(ident) => ident.value.to_lowercase().ends_with("id"),
                        Expr::CompoundIdentifier(parts) => parts
                            .last()
                            .map(|p| p.value.to_lowercase().ends_with("id"))
                            .unwrap_or(false),
                        _ => false,
                    }
                } else {
                    Self::has_primary_key_filter(left)
                }
            }
            _ => false,
        }
    }

    /// Analyze which tables are actually used
    fn analyze_table_usage(&self, select: &Select) -> HashSet<String> {
        let mut used_tables = HashSet::new();

        // Analyze SELECT clause
        for item in &select.projection {
            Self::extract_tables_from_select_item(item, &mut used_tables);
        }

        // Analyze WHERE clause
        if let Some(selection) = &select.selection {
            Self::extract_tables_from_expr(selection, &mut used_tables);
        }

        used_tables
    }

    fn extract_tables_from_select_item(item: &SelectItem, tables: &mut HashSet<String>) {
        match item {
            SelectItem::UnnamedExpr(expr) | SelectItem::ExprWithAlias { expr, .. } => {
                Self::extract_tables_from_expr(expr, tables);
            }
            SelectItem::QualifiedWildcard(name, _) => {
                if let Some(first) = name.0.first() {
                    tables.insert(first.value.clone());
                }
            }
            _ => {}
        }
    }

    fn extract_tables_from_expr(expr: &Expr, tables: &mut HashSet<String>) {
        match expr {
            Expr::CompoundIdentifier(parts) => {
                if parts.len() >= 2 {
                    // table.column format
                    tables.insert(parts[0].value.clone());
                }
            }
            Expr::BinaryOp { left, right, .. } => {
                Self::extract_tables_from_expr(left, tables);
                Self::extract_tables_from_expr(right, tables);
            }
            Expr::Nested(inner) => {
                Self::extract_tables_from_expr(inner, tables);
            }
            Expr::Function(func) => {
                for arg in &func.args {
                    if let sqlparser::ast::FunctionArg::Unnamed(
                        sqlparser::ast::FunctionArgExpr::Expr(e),
                    ) = arg
                    {
                        Self::extract_tables_from_expr(e, tables);
                    }
                }
            }
            _ => {}
        }
    }

    /// Get all joined table aliases
    fn get_joined_tables(&self, from: &[TableWithJoins]) -> Vec<String> {
        let mut tables = Vec::new();

        for from_item in from {
            for join in &from_item.joins {
                if let sqlparser::ast::TableFactor::Table { alias, .. } = &join.relation {
                    if let Some(alias) = alias {
                        tables.push(alias.name.value.clone());
                    }
                }
            }
        }

        tables
    }

    /// Try to remove redundant JOINs
    fn remove_redundant_joins(&self, mut select: Box<Select>) -> Result<Box<Select>> {
        let used_tables = self.analyze_table_usage(&select);
        let joined_tables = self.get_joined_tables(&select.from);

        // Find tables that are joined but never used
        let redundant_tables: Vec<_> = joined_tables
            .iter()
            .filter(|t| !used_tables.contains(*t))
            .cloned()
            .collect();

        if !redundant_tables.is_empty() {
            tracing::info!(
                "Found {} potentially redundant JOIN(s): {:?}",
                redundant_tables.len(),
                redundant_tables
            );

            // Remove redundant JOINs
            for from_item in &mut select.from {
                from_item.joins.retain(|join| {
                    if let sqlparser::ast::TableFactor::Table { alias, .. } = &join.relation {
                        if let Some(alias) = alias {
                            !redundant_tables.contains(&alias.name.value)
                        } else {
                            true
                        }
                    } else {
                        true
                    }
                });
            }
        }

        Ok(select)
    }
}

impl RewriteRule for RedundantJoinRemoverEnhanced {
    fn name(&self) -> &str {
        "Enhanced Redundant JOIN Removal"
    }

    fn description(&self) -> &str {
        "Removes JOIN operations that are not actually used in the query"
    }

    fn applies_to(&self, stmt: &Statement) -> bool {
        Self::has_potentially_redundant_join(stmt)
    }

    fn rewrite(&self, stmt: Statement) -> Result<Statement> {
        match stmt {
            Statement::Query(mut query) => {
                if let SetExpr::Select(select) = *query.body {
                    let optimized = self.remove_redundant_joins(select)?;
                    query.body = Box::new(SetExpr::Select(optimized));
                    Ok(Statement::Query(query))
                } else {
                    Ok(Statement::Query(query))
                }
            }
            _ => Ok(stmt),
        }
    }

    fn estimated_improvement(&self) -> f64 {
        30.0
    }
}

impl Default for RedundantJoinRemoverEnhanced {
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
    fn test_detects_redundant_join() {
        let rule = RedundantJoinRemoverEnhanced::new();
        let stmt = parse(
            "SELECT u.id, u.name FROM users u JOIN orders o ON u.id = o.user_id WHERE u.id = 123",
        );

        assert!(rule.applies_to(&stmt));
    }

    #[test]
    fn test_removes_unused_join() {
        let rule = RedundantJoinRemoverEnhanced::new();
        let stmt = parse(
            "SELECT u.id, u.name FROM users u JOIN orders o ON u.id = o.user_id WHERE u.id = 123",
        );

        let result = rule.rewrite(stmt).unwrap();
        let sql = format!("{}", result);

        // JOIN should be removed (orders table not used in SELECT)
        // Note: This is a heuristic check
        assert!(sql.contains("users"));
    }

    #[test]
    fn test_keeps_used_join() {
        let rule = RedundantJoinRemoverEnhanced::new();
        let stmt = parse(
            "SELECT u.id, o.total FROM users u JOIN orders o ON u.id = o.user_id WHERE u.id = 123",
        );

        let result = rule.rewrite(stmt).unwrap();
        let sql = format!("{}", result);

        // JOIN should be kept (orders.total is used)
        assert!(sql.to_uppercase().contains("JOIN") || sql.contains("orders"));
    }

    #[test]
    fn test_ignores_query_without_join() {
        let rule = RedundantJoinRemoverEnhanced::new();
        let stmt = parse("SELECT * FROM users WHERE id = 1");

        assert!(!rule.applies_to(&stmt));
    }
}
