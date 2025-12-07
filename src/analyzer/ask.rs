use crate::analyzer::ai_advisor::{LlmClient, OpenAiClient};
use crate::collector::DatabaseCollector;
use anyhow::{Context, Result};
use colored::*;

pub struct AskHandler {
    collector: DatabaseCollector,
    llm_client: OpenAiClient,
}

impl AskHandler {
    pub fn new(collector: DatabaseCollector, api_key: String, model: String) -> Self {
        Self {
            collector,
            llm_client: OpenAiClient::new(api_key, model),
        }
    }

    pub async fn handle(&self, question: &str) -> Result<()> {
        println!("{}", "Gathering database context...".dimmed());

        // 1. Collect Context
        // We use a small limit for query stats to avoid overwhelming the context window
        let query_stats = self
            .collector
            .collect_query_stats(10)
            .await
            .unwrap_or_else(|_| vec![]); // Graceful degradation if stats fail

        let conn_stats = self
            .collector
            .collect_connection_stats()
            .await
            .context("Failed to collect connection stats")?;

        let db_metrics = self
            .collector
            .collect_database_metrics()
            .await
            .unwrap_or_default();

        // 2. Build Prompt
        let mut context = String::new();
        context.push_str("Current Database State:\n");

        context.push_str(&format!(
            "- Active Connections: {}/{}\n",
            conn_stats.total_connections, conn_stats.max_connections
        ));
        context.push_str(&format!(
            "- Idle in Transaction: {}\n",
            conn_stats.idle_in_transaction_count
        ));
        context.push_str(&format!(
            "- Locked Connections: {}\n",
            conn_stats.locked_connections_count
        ));
        context.push_str(&format!(
            "- Buffer Cache Hit Ratio: {:.2}%\n",
            db_metrics.buffer_cache_hit_ratio
        ));
        context.push_str(&format!("- Total Deadlocks: {}\n", db_metrics.deadlocks));
        context.push_str(&format!(
            "- Temp Files Created: {} ({} bytes)\n",
            db_metrics.temp_files, db_metrics.temp_bytes
        ));

        if let Some(longest_idle) = conn_stats.oldest_idle_transaction_secs {
            context.push_str(&format!("- Longest Idle Tx: {}s\n", longest_idle));
        }

        if !query_stats.is_empty() {
            context.push_str("\nTop Slowest Queries (by mean time):\n");
            for (i, q) in query_stats.iter().take(5).enumerate() {
                let query_snippet: String = q
                    .query
                    .chars()
                    .map(|c| if c == '\n' { ' ' } else { c })
                    .take(150)
                    .collect();

                context.push_str(&format!(
                    "{}. [Mean: {:.2}ms, Calls: {}] {}\n",
                    i + 1,
                    q.mean_exec_time_ms,
                    q.calls,
                    query_snippet
                ));
            }
        }

        let prompt = format!(
            "You are a PostgreSQL expert assistant. \
            Here is the current database state:\n\n{}\n\n\
            User Question: {}\n\n\
            Answer the user's question based on the provided metrics. \
            If the metrics explain the issue, point it out. \
            If you need more info, say what to check next. \
            Keep the answer concise and actionable.",
            context, question
        );

        println!("{}", "Analyzing with AI...".dimmed());

        // 3. Get Answer
        // We use the complete method from LlmClient trait
        let answer = self.llm_client.complete(&prompt).await?;

        // 4. Print Answer
        println!("\n{}", "AI Analysis:".bold().green());
        println!("{}", answer);

        Ok(())
    }
}
