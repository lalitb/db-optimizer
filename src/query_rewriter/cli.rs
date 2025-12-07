// src/query_rewriter/cli.rs - CLI interface for query rewriter

use anyhow::{Context, Result};
use std::fs;
use std::io::{self, Read};

use crate::query_rewriter::{QueryRewriter, RewriteResult};

/// Input source for SQL queries
#[derive(Debug, Clone)]
pub enum InputSource {
    /// Read from a file
    File(String),

    /// Read from stdin
    Stdin,

    /// Direct SQL string
    Inline(String),
}

/// Output format for rewriter results
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFormat {
    /// Human-readable terminal output
    Terminal,

    /// JSON format for programmatic use
    Json,

    /// Markdown format for documentation
    Markdown,
}

impl OutputFormat {
    pub fn from_str(s: &str) -> Result<Self> {
        match s.to_lowercase().as_str() {
            "terminal" | "term" => Ok(Self::Terminal),
            "json" => Ok(Self::Json),
            "markdown" | "md" => Ok(Self::Markdown),
            _ => Err(anyhow::anyhow!(
                "Invalid format: {}. Use: terminal, json, markdown",
                s
            )),
        }
    }
}

/// Query rewriter CLI handler
pub struct RewriterCli {
    rewriter: QueryRewriter,
}

impl RewriterCli {
    /// Create a new rewriter CLI
    pub fn new() -> Self {
        Self {
            rewriter: QueryRewriter::new_enhanced(),
        }
    }

    /// Read SQL from input source
    pub fn read_input(&self, source: &InputSource) -> Result<String> {
        match source {
            InputSource::File(path) => {
                fs::read_to_string(path).with_context(|| format!("Failed to read file: {}", path))
            }
            InputSource::Stdin => {
                let mut buffer = String::new();
                io::stdin()
                    .read_to_string(&mut buffer)
                    .context("Failed to read from stdin")?;
                Ok(buffer)
            }
            InputSource::Inline(sql) => Ok(sql.clone()),
        }
    }

    /// Rewrite SQL and format output
    pub fn rewrite_and_format(
        &self,
        sql: &str,
        format: OutputFormat,
        include_explanations: bool,
    ) -> Result<String> {
        // Perform rewrite
        let result = self.rewriter.optimize(sql)?;

        // Format output
        match format {
            OutputFormat::Terminal => self.format_terminal(&result, include_explanations),
            OutputFormat::Json => self.format_json(&result),
            OutputFormat::Markdown => self.format_markdown(&result, include_explanations),
        }
    }

    /// Format output for terminal
    fn format_terminal(
        &self,
        result: &RewriteResult,
        include_explanations: bool,
    ) -> Result<String> {
        use colored::Colorize;

        let mut output = String::new();

        // Header
        output.push_str(&format!("{}\n", "Query Rewriter Results".bold().cyan()));
        output.push_str(&"=".repeat(60));
        output.push_str("\n\n");

        // Applied rules
        if !result.applied_rules.is_empty() {
            output.push_str(&format!("{}\n", "Applied Rules:".bold()));
            for rule in &result.applied_rules {
                output.push_str(&format!("  • {}\n", rule.green()));
            }
            output.push('\n');
        } else {
            output.push_str(&format!("{}\n\n", "No optimizations applied".yellow()));
        }

        // Original query
        output.push_str(&format!("{}\n", "Original Query:".bold()));
        output.push_str(&format!("{}\n\n", &result.original_sql));

        // Rewritten query
        if result.original_sql != result.optimized_sql {
            output.push_str(&format!("{}\n", "Optimized Query:".bold().green()));
            output.push_str(&format!("{}\n\n", &result.optimized_sql));
        }

        // Estimated improvement
        if result.estimated_improvement_pct > 0.0 {
            output.push_str(&format!(
                "{} {:.1}%\n\n",
                "Estimated Improvement:".bold(),
                result.estimated_improvement_pct
            ));
        }

        // Explanations
        if include_explanations && !result.reasoning.is_empty() {
            output.push_str(&format!("{}\n", "Reasoning:".bold()));
            for (i, explanation) in result.reasoning.iter().enumerate() {
                output.push_str(&format!("{}. {}\n", i + 1, explanation));
            }
            output.push('\n');
        }

        Ok(output)
    }

    /// Format output as JSON
    fn format_json(&self, result: &RewriteResult) -> Result<String> {
        let json_output = serde_json::json!({
            "was_rewritten": result.original_sql != result.optimized_sql,
            "original_query": result.original_sql,
            "optimized_query": result.optimized_sql,
            "rules_applied": result.applied_rules,
            "estimated_improvement_pct": result.estimated_improvement_pct,
            "reasoning": result.reasoning,
        });

        serde_json::to_string_pretty(&json_output).context("Failed to serialize to JSON")
    }

    /// Format output as Markdown
    fn format_markdown(
        &self,
        result: &RewriteResult,
        include_explanations: bool,
    ) -> Result<String> {
        let mut output = String::new();

        // Header
        output.push_str("# Query Optimization Report\n\n");

        // Status
        if result.original_sql != result.optimized_sql {
            output.push_str("**Status:** ✅ Optimized\n\n");
        } else {
            output.push_str("**Status:** ℹ️ No optimizations applied\n\n");
        }

        // Applied rules
        if !result.applied_rules.is_empty() {
            output.push_str("## Applied Optimizations\n\n");
            for rule in &result.applied_rules {
                output.push_str(&format!("- {}\n", rule));
            }
            output.push('\n');
        }

        // Estimated improvement
        if result.estimated_improvement_pct > 0.0 {
            output.push_str(&format!(
                "**Estimated Improvement:** {:.1}%\n\n",
                result.estimated_improvement_pct
            ));
        }

        // Original query
        output.push_str("## Original Query\n\n");
        output.push_str("```sql\n");
        output.push_str(&result.original_sql);
        output.push_str("\n```\n\n");

        // Optimized query
        if result.original_sql != result.optimized_sql {
            output.push_str("## Optimized Query\n\n");
            output.push_str("```sql\n");
            output.push_str(&result.optimized_sql);
            output.push_str("\n```\n\n");
        }

        // Reasoning
        if include_explanations && !result.reasoning.is_empty() {
            output.push_str("## Reasoning\n\n");
            for (i, explanation) in result.reasoning.iter().enumerate() {
                output.push_str(&format!("{}. {}\n", i + 1, explanation));
            }
            output.push('\n');
        }

        Ok(output)
    }
}

impl Default for RewriterCli {
    fn default() -> Self {
        Self::new()
    }
}

/// Convenience function for quick rewrites
pub fn rewrite_sql(sql: &str, format: OutputFormat) -> Result<String> {
    let cli = RewriterCli::new();
    cli.rewrite_and_format(sql, format, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_output_format_from_str() {
        assert_eq!(
            OutputFormat::from_str("terminal").unwrap(),
            OutputFormat::Terminal
        );
        assert_eq!(OutputFormat::from_str("json").unwrap(), OutputFormat::Json);
        assert_eq!(
            OutputFormat::from_str("markdown").unwrap(),
            OutputFormat::Markdown
        );
        assert!(OutputFormat::from_str("invalid").is_err());
    }

    #[test]
    fn test_rewrite_simple_query() {
        let cli = RewriterCli::new();
        let sql = "SELECT * FROM users WHERE id = 1";

        let result = cli.rewrite_and_format(sql, OutputFormat::Json, false);
        assert!(result.is_ok());
    }
}
