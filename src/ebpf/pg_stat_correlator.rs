// src/ebpf/pg_stat_correlator.rs - PostgreSQL pg_stat_activity correlation
//
// This module correlates eBPF PIDs with PostgreSQL backend queries by
// periodically polling pg_stat_activity. This bridges the gap between
// kernel-level PIDs and application-level query context.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use chrono::{DateTime, Utc};
use sqlx::PgPool;
use tokio::sync::RwLock;
use tracing::{debug, error, warn};

/// Query information from pg_stat_activity
#[derive(Debug, Clone)]
pub struct ActiveQuery {
    /// PostgreSQL backend PID
    pub pid: u32,
    /// Database name
    pub datname: String,
    /// User name
    pub usename: String,
    /// Application name
    pub application_name: String,
    /// Client address
    pub client_addr: Option<String>,
    /// Current query text
    pub query: String,
    /// Query start time
    pub query_start: Option<DateTime<Utc>>,
    /// State (active, idle, etc.)
    pub state: String,
    /// Wait event type
    pub wait_event_type: Option<String>,
    /// Wait event
    pub wait_event: Option<String>,
    /// Backend start time
    pub backend_start: Option<DateTime<Utc>>,
    /// Normalized query hash (computed)
    pub query_hash: u64,
}

/// Correlator that maps PIDs to active queries
pub struct PgStatCorrelator {
    pool: PgPool,
    /// Cache of active queries by PID
    active_queries: Arc<RwLock<HashMap<u32, ActiveQuery>>>,
    /// Prepared statement cache: (pid, portal_hash) -> query_hash
    prepared_statements: Arc<RwLock<HashMap<(u32, u64), u64>>>,
    /// Refresh interval
    refresh_interval: Duration,
}

impl PgStatCorrelator {
    /// Create a new correlator
    pub fn new(pool: PgPool) -> Self {
        Self::with_interval(pool, Duration::from_secs(1))
    }
    
    /// Create with custom refresh interval
    pub fn with_interval(pool: PgPool, interval: Duration) -> Self {
        Self {
            pool,
            active_queries: Arc::new(RwLock::new(HashMap::new())),
            prepared_statements: Arc::new(RwLock::new(HashMap::new())),
            refresh_interval: interval,
        }
    }
    
    /// Start the background refresh loop
    pub async fn run(&self) {
        let mut interval = tokio::time::interval(self.refresh_interval);
        
        loop {
            interval.tick().await;
            
            if let Err(e) = self.refresh().await {
                error!("Failed to refresh pg_stat_activity: {}", e);
            }
        }
    }
    
    /// Refresh the cache from pg_stat_activity
    pub async fn refresh(&self) -> Result<(), sqlx::Error> {
        let rows = sqlx::query_as::<_, PgStatActivityRow>(
            r#"
            SELECT 
                pid,
                datname,
                usename,
                application_name,
                client_addr::text,
                query,
                query_start,
                state,
                wait_event_type,
                wait_event,
                backend_start
            FROM pg_stat_activity
            WHERE pid != pg_backend_pid()
              AND state IS NOT NULL
              AND query IS NOT NULL
            "#
        )
        .fetch_all(&self.pool)
        .await?;
        
        let mut cache = self.active_queries.write().await;
        cache.clear();
        
        for row in rows {
            let query_str = row.query.as_deref().unwrap_or("");
            let query_hash = compute_query_hash(query_str);
            
            let active = ActiveQuery {
                pid: row.pid as u32,
                datname: row.datname.unwrap_or_default(),
                usename: row.usename.unwrap_or_default(),
                application_name: row.application_name.unwrap_or_default(),
                client_addr: row.client_addr,
                query: row.query.unwrap_or_default(),
                query_start: row.query_start,
                state: row.state.unwrap_or_default(),
                wait_event_type: row.wait_event_type,
                wait_event: row.wait_event,
                backend_start: row.backend_start,
                query_hash,
            };
            
            cache.insert(active.pid, active);
        }
        
        debug!("Refreshed pg_stat_activity: {} active backends", cache.len());
        Ok(())
    }
    
    /// Get query info for a PID
    pub async fn get_query(&self, pid: u32) -> Option<ActiveQuery> {
        let cache = self.active_queries.read().await;
        cache.get(&pid).cloned()
    }
    
    /// Get query hash for a PID
    pub async fn get_query_hash(&self, pid: u32) -> Option<u64> {
        let cache = self.active_queries.read().await;
        cache.get(&pid).map(|q| q.query_hash)
    }
    
    /// Get all active PIDs
    pub async fn get_active_pids(&self) -> Vec<u32> {
        let cache = self.active_queries.read().await;
        cache.keys().cloned().collect()
    }
    
    /// Check if a PID is currently running a query
    pub async fn is_active(&self, pid: u32) -> bool {
        let cache = self.active_queries.read().await;
        cache.get(&pid)
            .map(|q| q.state == "active")
            .unwrap_or(false)
    }
    
    /// Get wait event for a PID (for lock correlation)
    pub async fn get_wait_event(&self, pid: u32) -> Option<(String, String)> {
        let cache = self.active_queries.read().await;
        cache.get(&pid).and_then(|q| {
            match (&q.wait_event_type, &q.wait_event) {
                (Some(t), Some(e)) => Some((t.clone(), e.clone())),
                _ => None,
            }
        })
    }
    
    /// Register a prepared statement mapping
    /// Called when we see a Parse message in the eBPF trace
    pub async fn register_prepared_statement(&self, pid: u32, portal_hash: u64, query_hash: u64) {
        let mut cache = self.prepared_statements.write().await;
        cache.insert((pid, portal_hash), query_hash);
        
        // Limit cache size
        if cache.len() > 10000 {
            // Simple eviction - remove oldest entries
            let keys: Vec<_> = cache.keys().take(1000).cloned().collect();
            for key in keys {
                cache.remove(&key);
            }
        }
    }
    
    /// Look up query hash for a prepared statement execution
    pub async fn get_prepared_query_hash(&self, pid: u32, portal_hash: u64) -> Option<u64> {
        let cache = self.prepared_statements.read().await;
        cache.get(&(pid, portal_hash)).cloned()
    }
    
    /// Get queries waiting on a specific blocking PID
    pub async fn get_blocked_by(&self, blocking_pid: u32) -> Vec<ActiveQuery> {
        // This requires pg_blocking_pids() which may not be available in all versions
        let result = sqlx::query_as::<_, PgStatActivityRow>(
            r#"
            SELECT 
                pid,
                datname,
                usename,
                application_name,
                client_addr::text,
                query,
                query_start,
                state,
                wait_event_type,
                wait_event,
                backend_start
            FROM pg_stat_activity
            WHERE $1 = ANY(pg_blocking_pids(pid))
            "#
        )
        .bind(blocking_pid as i32)
        .fetch_all(&self.pool)
        .await;
        
        match result {
            Ok(rows) => {
                rows.into_iter().map(|row| {
                    let query_hash = compute_query_hash(&row.query.as_deref().unwrap_or(""));
                    ActiveQuery {
                        pid: row.pid as u32,
                        datname: row.datname.unwrap_or_default(),
                        usename: row.usename.unwrap_or_default(),
                        application_name: row.application_name.unwrap_or_default(),
                        client_addr: row.client_addr,
                        query: row.query.unwrap_or_default(),
                        query_start: row.query_start,
                        state: row.state.unwrap_or_default(),
                        wait_event_type: row.wait_event_type,
                        wait_event: row.wait_event,
                        backend_start: row.backend_start,
                        query_hash,
                    }
                }).collect()
            }
            Err(e) => {
                warn!("Failed to get blocked queries: {}", e);
                Vec::new()
            }
        }
    }
}

/// Row from pg_stat_activity
#[derive(sqlx::FromRow)]
struct PgStatActivityRow {
    pid: i32,
    datname: Option<String>,
    usename: Option<String>,
    application_name: Option<String>,
    client_addr: Option<String>,
    query: Option<String>,
    query_start: Option<DateTime<Utc>>,
    state: Option<String>,
    wait_event_type: Option<String>,
    wait_event: Option<String>,
    backend_start: Option<DateTime<Utc>>,
}

/// Compute a hash for query normalization
/// 
/// This is a simplified version - for production, use proper query normalization
fn compute_query_hash(query: &str) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    
    // Simple normalization: lowercase, collapse whitespace, remove literals
    let normalized = normalize_query(query);
    
    let mut hasher = DefaultHasher::new();
    normalized.hash(&mut hasher);
    hasher.finish()
}

/// Normalize a query for consistent hashing
fn normalize_query(query: &str) -> String {
    let mut result = query.to_lowercase();
    
    // Collapse whitespace
    result = result.split_whitespace().collect::<Vec<_>>().join(" ");
    
    // Remove numeric literals (simple regex-like replacement)
    let mut normalized = String::with_capacity(result.len());
    let mut in_number = false;
    let mut in_string = false;
    let mut prev_char = ' ';
    
    for c in result.chars() {
        if c == '\'' {
            if !in_string {
                in_string = true;
                normalized.push('?');
            } else if prev_char != '\\' {
                in_string = false;
            }
        } else if in_string {
            // Skip string contents
        } else if c.is_ascii_digit() {
            if !in_number {
                in_number = true;
                normalized.push('?');
            }
            // Skip additional digits
        } else {
            in_number = false;
            normalized.push(c);
        }
        prev_char = c;
    }
    
    normalized
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_normalize_query() {
        assert_eq!(
            normalize_query("SELECT * FROM users WHERE id = 123"),
            "select * from users where id = ?"
        );
        
        assert_eq!(
            normalize_query("SELECT * FROM users WHERE name = 'John'"),
            "select * from users where name = ?"
        );
        
        assert_eq!(
            normalize_query("SELECT * FROM orders WHERE total > 100.50"),
            "select * from orders where total > ?.?"
        );
    }
    
    #[test]
    fn test_query_hash_consistency() {
        let q1 = "SELECT * FROM users WHERE id = 123";
        let q2 = "SELECT * FROM users WHERE id = 456";
        let q3 = "SELECT * FROM orders WHERE id = 123";
        
        // Same query with different params should have same hash
        assert_eq!(compute_query_hash(q1), compute_query_hash(q2));
        
        // Different queries should have different hashes
        assert_ne!(compute_query_hash(q1), compute_query_hash(q3));
    }
}
