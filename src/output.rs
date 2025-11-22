use crate::models::*;
use colored::*;
use anyhow::Result;

pub struct OutputFormatter;

impl OutputFormatter {
    /// Format report for terminal display
    pub fn format_terminal(report: &AnalysisReport) -> String {
        let mut output = String::new();
        
        // Header
        output.push_str(&format!("\n{}\n", "━".repeat(70).bright_blue()));
        output.push_str(&format!("{}\n", "PostgreSQL Query Health Advisor".bright_cyan().bold()));
        output.push_str(&format!("{}\n", "━".repeat(70).bright_blue()));
        output.push_str(&format!("Database: {}\n", report.database_name.bright_white()));
        output.push_str(&format!("Analyzed: {}\n", report.analyzed_at.format("%Y-%m-%d %H:%M:%S UTC")));
        output.push_str(&format!("PostgreSQL: {}\n", report.postgres_version));
        
        // Health Score
        let score = report.health_score;
        let score_color = if score >= 90 {
            score.to_string().green().bold()
        } else if score >= 70 {
            score.to_string().yellow().bold()
        } else {
            score.to_string().red().bold()
        };
        output.push_str(&format!("Health Score: {}/100\n", score_color));
        
        // Summary box
        output.push_str(&format!("\n{}\n", "SUMMARY".bright_yellow().bold()));
        output.push_str(&format!("{}\n", "━".repeat(70).bright_blue()));
        output.push_str(&format!("  {} Issues Found\n", report.summary.issues_found.to_string().bright_red().bold()));
        
        if report.summary.critical_issues > 0 || report.summary.high_priority_issues > 0 {
            output.push_str(&format!("  {} Critical  ", "●".bright_red()));
            output.push_str(&format!("{} High Priority\n", "●".yellow()));
        }
        
        output.push_str(&format!("\n  {} Slow Queries\n", report.summary.slow_queries));
        output.push_str(&format!("  {} Missing Indexes Suggested\n", report.summary.missing_indexes_suggested));
        output.push_str(&format!("  {} Unused Indexes Found\n", report.summary.unused_indexes_found));
        
        output.push_str(&format!("\n  💰 Potential Savings: ${:.0}/month\n", 
            report.summary.total_estimated_savings_usd_monthly));
        
        // Top recommendations
        if !report.recommendations.is_empty() {
            output.push_str(&format!("\n{}\n", "TOP RECOMMENDATIONS".bright_yellow().bold()));
            output.push_str(&format!("{}\n", "━".repeat(70).bright_blue()));
            
            for (i, rec) in report.recommendations.iter().take(10).enumerate() {
                output.push_str(&Self::format_recommendation(rec, i + 1));
                output.push_str("\n");
            }
            
            if report.recommendations.len() > 10 {
                output.push_str(&format!("\n... and {} more recommendations\n", 
                    report.recommendations.len() - 10));
            }
        }
        
        // Footer
        output.push_str(&format!("\n{}\n", "━".repeat(70).bright_blue()));
        output.push_str(&format!("{}\n", "Export full report: --format json --output report.json".bright_black()));
        
        output
    }
    
    fn format_recommendation(rec: &Recommendation, index: usize) -> String {
        let mut output = String::new();
        
        let severity_badge = match rec.severity {
            Severity::Critical => "CRITICAL".bright_red().bold(),
            Severity::High => "HIGH".yellow().bold(),
            Severity::Medium => "MEDIUM".blue(),
            Severity::Low => "LOW".bright_black(),
        };
        
        output.push_str(&format!("\n{}. [{}] {}\n", 
            index, severity_badge, rec.title.bright_white().bold()));
        
        output.push_str(&format!("   {}\n", rec.description));
        
        if let Some(savings) = rec.estimated_cost_savings_usd_monthly {
            if savings > 0.0 {
                output.push_str(&format!("   💰 Est. savings: ${:.0}/month\n", savings));
            }
        }
        
        if let Some(queries) = rec.affected_queries_count {
            output.push_str(&format!("   📊 Affects {} queries\n", queries));
        }
        
        output.push_str(&format!("\n   {}\n", "Migration SQL:".bright_cyan()));
        output.push_str(&format!("   {}\n", rec.migration_sql.bright_white()));
        
        if let Some(rollback) = &rec.rollback_sql {
            output.push_str(&format!("   {}\n", "Rollback:".bright_black()));
            output.push_str(&format!("   {}\n", rollback.bright_black()));
        }
        
        output.push_str(&format!("   ℹ️  {}\n", rec.safety_notes.bright_black()));
        
        output
    }
    
    /// Format report as JSON
    pub fn format_json(report: &AnalysisReport) -> Result<String> {
        Ok(serde_json::to_string_pretty(report)?)
    }
    
    /// Format report as Markdown
    pub fn format_markdown(report: &AnalysisReport) -> String {
        let mut output = String::new();
        
        output.push_str("# PostgreSQL Query Health Report\n\n");
        output.push_str(&format!("**Database:** {}\n", report.database_name));
        output.push_str(&format!("**Analyzed:** {}\n", report.analyzed_at));
        output.push_str(&format!("**PostgreSQL Version:** {}\n\n", report.postgres_version));
        
        output.push_str("## Summary\n\n");
        output.push_str(&format!("- **Issues Found:** {}\n", report.summary.issues_found));
        output.push_str(&format!("- **Potential Monthly Savings:** ${:.0}\n", 
            report.summary.total_estimated_savings_usd_monthly));
        output.push_str(&format!("- Slow Queries: {}\n", report.summary.slow_queries));
        output.push_str(&format!("- Missing Indexes: {}\n", report.summary.missing_indexes_suggested));
        output.push_str(&format!("- Unused Indexes: {}\n\n", report.summary.unused_indexes_found));
        
        output.push_str("## Recommendations\n\n");
        
        for (i, rec) in report.recommendations.iter().enumerate() {
            output.push_str(&format!("### {}. [{}] {}\n\n", 
                i + 1, rec.severity.as_str(), rec.title));
            output.push_str(&format!("{}\n\n", rec.description));
            
            if let Some(savings) = rec.estimated_cost_savings_usd_monthly {
                output.push_str(&format!("**Estimated Savings:** ${:.0}/month\n\n", savings));
            }
            
            output.push_str("**Migration SQL:**\n```sql\n");
            output.push_str(&format!("{}\n", rec.migration_sql));
            output.push_str("```\n\n");
            
            if let Some(rollback) = &rec.rollback_sql {
                output.push_str("**Rollback:**\n```sql\n");
                output.push_str(&format!("{}\n", rollback));
                output.push_str("```\n\n");
            }
            
            output.push_str(&format!("**Safety:** {}\n\n", rec.safety_notes));
            output.push_str("---\n\n");
        }
        
        output
    }
}
