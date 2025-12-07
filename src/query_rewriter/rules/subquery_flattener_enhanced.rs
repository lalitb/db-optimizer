use crate::query_rewriter::RewriteRule;
use anyhow::Result;
use sqlparser::ast::{
    Expr, Ident, Join, JoinConstraint, JoinOperator, Query, Select, SelectItem, SetExpr, Statement,
    TableFactor,
};

/// Enhanced Subquery Flattener with actual JOIN conversion
///
/// Converts IN (SELECT...) subqueries into JOIN operations.
/// This implementation handles simple cases where the subquery selects a single column.
pub struct SubqueryFlattenerEnhanced;

impl SubqueryFlattenerEnhanced {
    pub fn new() -> Self {
        Self
    }

    fn has_simple_in_subquery(stmt: &Statement) -> bool {
        match stmt {
            Statement::Query(query) => {
                if let SetExpr::Select(select) = &*query.body {
                    if let Some(selection) = &select.selection {
                        Self::contains_simple_in_subquery(selection)
                    } else {
                        false
                    }
                } else {
                    false
                }
            }
            _ => false,
        }
    }

    fn contains_simple_in_subquery(expr: &Expr) -> bool {
        match expr {
            Expr::InSubquery { subquery, .. } => {
                // Check if subquery is simple (single column SELECT)
                if let SetExpr::Select(select) = &*subquery.body {
                    // Simple case: SELECT single_column FROM table WHERE...
                    select.projection.len() == 1 && select.from.len() == 1
                } else {
                    false
                }
            }
            Expr::BinaryOp { left, right, .. } => {
                Self::contains_simple_in_subquery(left) || Self::contains_simple_in_subquery(right)
            }
            _ => false,
        }
    }

    /// Convert IN subquery to JOIN
    fn flatten_in_subquery(&self, mut outer_select: Box<Select>) -> Result<Box<Select>> {
        if let Some(selection) = &outer_select.selection {
            if let Some((left_col, subquery_info)) = Self::extract_in_subquery(selection) {
                // Add the subquery table as a JOIN
                if let Some(from_item) = outer_select.from.first_mut() {
                    let join = Join {
                        relation: TableFactor::Derived {
                            lateral: false,
                            subquery: Box::new(subquery_info.subquery.clone()),
                            alias: Some(sqlparser::ast::TableAlias {
                                name: Ident::new("_sub"),
                                columns: vec![],
                            }),
                        },
                        join_operator: JoinOperator::Inner(JoinConstraint::On(Expr::BinaryOp {
                            left: Box::new(left_col.clone()),
                            op: sqlparser::ast::BinaryOperator::Eq,
                            right: Box::new(Expr::CompoundIdentifier(vec![
                                Ident::new("_sub"),
                                subquery_info.column_name.clone(),
                            ])),
                        })),
                    };

                    from_item.joins.push(join);

                    // Remove the IN clause from WHERE
                    // For simplicity, if WHERE only has the IN clause, remove it entirely
                    // Otherwise keep the WHERE as-is
                    if Self::is_only_in_subquery(selection) {
                        outer_select.selection = None;
                    }

                    // Add DISTINCT if needed (IN semantics require distinct results)
                    if outer_select.distinct.is_none() {
                        outer_select.distinct = Some(sqlparser::ast::Distinct::Distinct);
                    }
                }
            }
        }

        Ok(outer_select)
    }

    fn extract_in_subquery(expr: &Expr) -> Option<(Expr, SubqueryInfo)> {
        match expr {
            Expr::InSubquery {
                expr: left,
                subquery,
                negated,
            } if !negated => {
                // Extract column name from subquery projection
                if let SetExpr::Select(select) = &*subquery.body {
                    if let Some(SelectItem::UnnamedExpr(Expr::Identifier(col_name))) =
                        select.projection.first()
                    {
                        return Some((
                            (**left).clone(),
                            SubqueryInfo {
                                subquery: (**subquery).clone(),
                                column_name: col_name.clone(),
                            },
                        ));
                    }
                }
                None
            }
            _ => None,
        }
    }

    fn is_only_in_subquery(expr: &Expr) -> bool {
        matches!(expr, Expr::InSubquery { .. })
    }
}

struct SubqueryInfo {
    subquery: Query,
    column_name: Ident,
}

impl RewriteRule for SubqueryFlattenerEnhanced {
    fn name(&self) -> &str {
        "Enhanced Subquery Flattening"
    }

    fn description(&self) -> &str {
        "Converts IN (SELECT...) to JOIN for better performance"
    }

    fn applies_to(&self, stmt: &Statement) -> bool {
        Self::has_simple_in_subquery(stmt)
    }

    fn rewrite(&self, stmt: Statement) -> Result<Statement> {
        match stmt {
            Statement::Query(mut query) => {
                if let SetExpr::Select(select) = *query.body {
                    let flattened = self.flatten_in_subquery(select)?;
                    query.body = Box::new(SetExpr::Select(flattened));
                    Ok(Statement::Query(query))
                } else {
                    Ok(Statement::Query(query))
                }
            }
            _ => Ok(stmt),
        }
    }

    fn estimated_improvement(&self) -> f64 {
        40.0
    }
}

impl Default for SubqueryFlattenerEnhanced {
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
    fn test_detects_simple_in_subquery() {
        let rule = SubqueryFlattenerEnhanced::new();
        let stmt = parse("SELECT * FROM users WHERE id IN (SELECT user_id FROM orders)");

        assert!(rule.applies_to(&stmt));
    }

    #[test]
    fn test_flattens_to_join() {
        let rule = SubqueryFlattenerEnhanced::new();
        let stmt = parse("SELECT * FROM users WHERE id IN (SELECT user_id FROM orders)");

        let result = rule.rewrite(stmt).unwrap();
        let sql = format!("{}", result);

        // Verify JOIN was added
        assert!(sql.to_uppercase().contains("JOIN"));
        assert!(sql.to_uppercase().contains("DISTINCT"));
    }

    #[test]
    fn test_ignores_complex_subquery() {
        let rule = SubqueryFlattenerEnhanced::new();
        let stmt = parse(
            "SELECT * FROM users WHERE id IN (SELECT user_id, COUNT(*) FROM orders GROUP BY user_id)"
        );

        // Should not apply to complex subqueries
        assert!(!rule.applies_to(&stmt));
    }

    #[test]
    fn test_ignores_simple_query() {
        let rule = SubqueryFlattenerEnhanced::new();
        let stmt = parse("SELECT * FROM users WHERE id = 1");

        assert!(!rule.applies_to(&stmt));
    }
}
