pub mod cli;
pub mod rules;

use anyhow::{Context, Result};
use sqlparser::ast::Statement;
use sqlparser::dialect::PostgreSqlDialect;
use sqlparser::parser::Parser;

#[derive(Debug, Clone)]
pub struct RewriteResult {
    pub original_sql: String,
    pub optimized_sql: String,
    pub applied_rules: Vec<String>,
    pub estimated_improvement_pct: f64,
    pub reasoning: Vec<String>,
}

pub trait RewriteRule: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn applies_to(&self, stmt: &Statement) -> bool;
    fn rewrite(&self, stmt: Statement) -> Result<Statement>;
    fn estimated_improvement(&self) -> f64;
}

pub struct QueryRewriter {
    rules: Vec<Box<dyn RewriteRule>>,
    dialect: PostgreSqlDialect,
}

impl QueryRewriter {
    pub fn new() -> Self {
        Self {
            rules: vec![
                Box::new(rules::LimitAdder::new()),
                Box::new(rules::SelectStarRemover::new()),
                Box::new(rules::PredicatePushdown::new()),
                Box::new(rules::SubqueryFlattener::new()),
                Box::new(rules::RedundantJoinRemover::new()),
            ],
            dialect: PostgreSqlDialect {},
        }
    }

    /// Create rewriter with enhanced rules that perform actual rewriting
    pub fn new_enhanced() -> Self {
        Self {
            rules: vec![
                Box::new(rules::LimitAdder::new()),
                Box::new(rules::SelectStarRemover::new()),
                Box::new(rules::PredicatePushdownEnhanced::new()),
                Box::new(rules::SubqueryFlattenerEnhanced::new()),
                Box::new(rules::RedundantJoinRemover::new()),
            ],
            dialect: PostgreSqlDialect {},
        }
    }

    /// Create rewriter with ALL enhanced rules (requires database pool for SELECT * removal)
    pub fn new_fully_enhanced(pool: sqlx::PgPool) -> Self {
        Self {
            rules: vec![
                Box::new(rules::LimitAdder::new()),
                Box::new(rules::SelectStarRemoverEnhanced::with_pool(pool)),
                Box::new(rules::PredicatePushdownEnhanced::new()),
                Box::new(rules::SubqueryFlattenerEnhanced::new()),
                Box::new(rules::RedundantJoinRemoverEnhanced::new()),
            ],
            dialect: PostgreSqlDialect {},
        }
    }

    pub fn with_custom_rules(mut self, rules: Vec<Box<dyn RewriteRule>>) -> Self {
        self.rules = rules;
        self
    }

    pub fn add_rule(&mut self, rule: Box<dyn RewriteRule>) {
        self.rules.push(rule);
    }

    /// Optimize a SQL query by applying rewrite rules
    pub fn optimize(&self, sql: &str) -> Result<RewriteResult> {
        tracing::debug!("Starting query optimization for: {}", sql);

        // Parse SQL
        let ast = Parser::parse_sql(&self.dialect, sql).context("Failed to parse SQL")?;

        if ast.is_empty() {
            return Ok(RewriteResult {
                original_sql: sql.to_string(),
                optimized_sql: sql.to_string(),
                applied_rules: vec![],
                estimated_improvement_pct: 0.0,
                reasoning: vec!["No statements to optimize".to_string()],
            });
        }

        let mut optimized_stmt = ast[0].clone();
        let mut applied_rules = Vec::new();
        let mut reasoning = Vec::new();
        let mut total_improvement = 0.0;

        // Apply each rule
        for rule in &self.rules {
            if rule.applies_to(&optimized_stmt) {
                tracing::info!("Applying rule: {}", rule.name());

                match rule.rewrite(optimized_stmt.clone()) {
                    Ok(rewritten) => {
                        optimized_stmt = rewritten;
                        applied_rules.push(rule.name().to_string());
                        reasoning.push(format!(
                            "{}: {} (est. {}% improvement)",
                            rule.name(),
                            rule.description(),
                            rule.estimated_improvement()
                        ));

                        // Compound improvements (not additive)
                        let remaining = 100.0 - total_improvement;
                        total_improvement += (remaining * rule.estimated_improvement()) / 100.0;
                    }
                    Err(e) => {
                        tracing::warn!("Failed to apply rule {}: {}", rule.name(), e);
                        reasoning.push(format!("{}: Skipped due to error: {}", rule.name(), e));
                    }
                }
            }
        }

        let optimized_sql = format!("{}", optimized_stmt);

        Ok(RewriteResult {
            original_sql: sql.to_string(),
            optimized_sql,
            applied_rules,
            estimated_improvement_pct: total_improvement,
            reasoning,
        })
    }

    /// Check if a query would benefit from optimization
    pub fn should_optimize(&self, sql: &str) -> bool {
        if let Ok(ast) = Parser::parse_sql(&self.dialect, sql) {
            if ast.is_empty() {
                return false;
            }

            // Check if any rule applies
            self.rules.iter().any(|rule| rule.applies_to(&ast[0]))
        } else {
            false
        }
    }
}

impl Default for QueryRewriter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_query() {
        let rewriter = QueryRewriter::new();
        let sql = "SELECT id, name FROM users WHERE id = 1";

        let result = rewriter.optimize(sql);
        assert!(result.is_ok());
    }

    #[test]
    fn test_should_optimize() {
        let rewriter = QueryRewriter::new();

        // Unbounded query should be optimizable
        let sql1 = "SELECT * FROM huge_table";
        assert!(rewriter.should_optimize(sql1));

        // Already optimized query
        let sql2 = "SELECT id FROM users LIMIT 10";
        // May or may not need optimization depending on rules
        let _ = rewriter.should_optimize(sql2);
    }

    #[test]
    fn test_invalid_sql() {
        let rewriter = QueryRewriter::new();
        let sql = "SELECT FROM WHERE";

        let result = rewriter.optimize(sql);
        assert!(result.is_err());
    }
}
