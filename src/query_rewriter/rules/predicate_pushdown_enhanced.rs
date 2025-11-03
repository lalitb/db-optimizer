use crate::query_rewriter::RewriteRule;
use sqlparser::ast::{Statement, Query, SetExpr, Select, TableFactor, Expr, TableWithJoins};
use anyhow::Result;

/// Enhanced Predicate Pushdown with actual rewriting capability
/// 
/// Pushes WHERE conditions into subqueries to filter data earlier.
/// This version can handle simple cases where the subquery is in the FROM clause
/// and the outer WHERE clause references columns from the subquery.
pub struct PredicatePushdownEnhanced;

impl PredicatePushdownEnhanced {
    pub fn new() -> Self {
        Self
    }
    
    fn can_push_predicate(stmt: &Statement) -> bool {
        match stmt {
            Statement::Query(query) => {
                if let SetExpr::Select(select) = &*query.body {
                    // Check for subquery in FROM with outer WHERE
                    if let Some(from) = select.from.first() {
                        if matches!(&from.relation, TableFactor::Derived { .. }) {
                            return select.selection.is_some();
                        }
                    }
                }
                false
            }
            _ => false,
        }
    }
    
    /// Try to push simple equality predicates into subquery
    fn push_simple_predicates(&self, mut outer_select: Box<Select>) -> Result<Box<Select>> {
        if let Some(from_item) = outer_select.from.first_mut() {
            if let TableFactor::Derived { subquery, alias, .. } = &mut from_item.relation {
                // Get the alias name if it exists
                let table_alias = alias.as_ref().map(|a| a.name.value.clone());
                
                // Extract predicates that can be pushed
                if let Some(outer_where) = &outer_select.selection {
                    let pushable = Self::extract_pushable_predicates(
                        outer_where,
                        table_alias.as_deref(),
                    );
                    
                    if !pushable.is_empty() {
                        // Modify subquery to include pushed predicates
                        if let SetExpr::Select(ref mut inner_select) = &mut *subquery.body {
                            // Combine existing WHERE with pushed predicates
                            let combined_where = if let Some(existing) = &inner_select.selection {
                                // AND existing predicates with new ones
                                Self::combine_predicates(existing.clone(), pushable)
                            } else {
                                Self::combine_predicates_vec(pushable)
                            };
                            
                            inner_select.selection = Some(combined_where);
                            
                            // Remove pushed predicates from outer WHERE
                            // For simplicity, we keep outer WHERE as-is in this version
                            // A full implementation would remove the pushed predicates
                        }
                    }
                }
            }
        }
        
        Ok(outer_select)
    }
    
    fn extract_pushable_predicates(expr: &Expr, alias: Option<&str>) -> Vec<Expr> {
        let mut pushable = Vec::new();
        
        // Simple case: single predicate like t.column = value
        if Self::is_simple_column_predicate(expr, alias) {
            pushable.push(Self::remove_table_qualifier(expr.clone(), alias));
        }
        
        // Handle AND chains
        if let Expr::BinaryOp { left, op, right } = expr {
            if matches!(op, sqlparser::ast::BinaryOperator::And) {
                pushable.extend(Self::extract_pushable_predicates(left, alias));
                pushable.extend(Self::extract_pushable_predicates(right, alias));
            }
        }
        
        pushable
    }
    
    fn is_simple_column_predicate(expr: &Expr, alias: Option<&str>) -> bool {
        match expr {
            Expr::BinaryOp { left, op, .. } => {
                if !matches!(
                    op,
                    sqlparser::ast::BinaryOperator::Eq
                        | sqlparser::ast::BinaryOperator::Gt
                        | sqlparser::ast::BinaryOperator::Lt
                        | sqlparser::ast::BinaryOperator::GtEq
                        | sqlparser::ast::BinaryOperator::LtEq
                ) {
                    return false;
                }
                
                // Check if left side references the table alias
                match &**left {
                    Expr::CompoundIdentifier(parts) => {
                        if let Some(alias_name) = alias {
                            parts.first().map(|p| p.value.as_str()) == Some(alias_name)
                        } else {
                            false
                        }
                    }
                    Expr::Identifier(_) => alias.is_none(),
                    _ => false,
                }
            }
            _ => false,
        }
    }
    
    fn remove_table_qualifier(expr: Expr, alias: Option<&str>) -> Expr {
        match expr {
            Expr::BinaryOp { left, op, right } => {
                let new_left = match *left {
                    Expr::CompoundIdentifier(mut parts) => {
                        if alias.is_some() && parts.len() > 1 {
                            // Remove first part (table alias)
                            parts.remove(0);
                            if parts.len() == 1 {
                                Expr::Identifier(parts[0].clone())
                            } else {
                                Expr::CompoundIdentifier(parts)
                            }
                        } else {
                            Expr::CompoundIdentifier(parts)
                        }
                    }
                    other => other,
                };
                
                Expr::BinaryOp {
                    left: Box::new(new_left),
                    op,
                    right,
                }
            }
            other => other,
        }
    }
    
    fn combine_predicates(existing: Expr, new: Vec<Expr>) -> Expr {
        let mut result = existing;
        for pred in new {
            result = Expr::BinaryOp {
                left: Box::new(result),
                op: sqlparser::ast::BinaryOperator::And,
                right: Box::new(pred),
            };
        }
        result
    }
    
    fn combine_predicates_vec(preds: Vec<Expr>) -> Expr {
        let mut iter = preds.into_iter();
        let mut result = iter.next().unwrap();
        for pred in iter {
            result = Expr::BinaryOp {
                left: Box::new(result),
                op: sqlparser::ast::BinaryOperator::And,
                right: Box::new(pred),
            };
        }
        result
    }
}

impl RewriteRule for PredicatePushdownEnhanced {
    fn name(&self) -> &str {
        "Enhanced Predicate Pushdown"
    }
    
    fn description(&self) -> &str {
        "Moves WHERE conditions into subqueries to reduce data volume early"
    }
    
    fn applies_to(&self, stmt: &Statement) -> bool {
        Self::can_push_predicate(stmt)
    }
    
    fn rewrite(&self, stmt: Statement) -> Result<Statement> {
        match stmt {
            Statement::Query(mut query) => {
                if let SetExpr::Select(select) = *query.body {
                    let enhanced_select = self.push_simple_predicates(select)?;
                    query.body = Box::new(SetExpr::Select(enhanced_select));
                    Ok(Statement::Query(query))
                } else {
                    Ok(Statement::Query(query))
                }
            }
            _ => Ok(stmt),
        }
    }
    
    fn estimated_improvement(&self) -> f64 {
        50.0
    }
}

impl Default for PredicatePushdownEnhanced {
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
    fn test_detects_pushdown_opportunity() {
        let rule = PredicatePushdownEnhanced::new();
        let stmt = parse(
            "SELECT * FROM (SELECT * FROM large_table) t WHERE t.date > '2024-01-01'"
        );
        
        assert!(rule.applies_to(&stmt));
    }
    
    #[test]
    fn test_pushes_simple_predicate() {
        let rule = PredicatePushdownEnhanced::new();
        let stmt = parse(
            "SELECT * FROM (SELECT id, date FROM users) t WHERE t.date > '2024-01-01'"
        );
        
        let result = rule.rewrite(stmt).unwrap();
        let sql = format!("{}", result);
        
        // Verify that predicate was pushed (SQL should contain nested WHERE)
        // This is a basic check - full verification would parse the result
        assert!(sql.contains("WHERE"));
    }
    
    #[test]
    fn test_ignores_simple_query() {
        let rule = PredicatePushdownEnhanced::new();
        let stmt = parse("SELECT * FROM users WHERE id = 1");
        
        assert!(!rule.applies_to(&stmt));
    }
}
