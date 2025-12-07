//! Session-Aware Query Tracking
//!
//! Tracks queries across connection pools and prepared statements.
//! Handles the PostgreSQL extended query protocol (Parse/Bind/Execute).

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tracing::{debug, trace, warn};

/// Session state for a PostgreSQL backend process
#[derive(Debug, Clone)]
pub struct SessionState {
    /// PostgreSQL backend PID
    pub backend_pid: u32,
    /// Current query hash (if executing)
    pub current_query_hash: Option<u64>,
    /// Whether current execution is a prepared statement
    pub is_prepared: bool,
    /// Current prepared statement name (if any)
    pub prepared_stmt_name: Option<String>,
    /// Session start time
    pub session_start: Instant,
    /// Last activity time
    pub last_activity: Instant,
    /// Total queries executed in this session
    pub query_count: u64,
    /// Application name (from pg_stat_activity)
    pub application_name: Option<String>,
    /// Client address
    pub client_addr: Option<String>,
    /// Database name
    pub database: Option<String>,
    /// Username
    pub username: Option<String>,
}

impl SessionState {
    pub fn new(backend_pid: u32) -> Self {
        let now = Instant::now();
        Self {
            backend_pid,
            current_query_hash: None,
            is_prepared: false,
            prepared_stmt_name: None,
            session_start: now,
            last_activity: now,
            query_count: 0,
            application_name: None,
            client_addr: None,
            database: None,
            username: None,
        }
    }

    /// Mark query start
    pub fn start_query(&mut self, query_hash: u64, is_prepared: bool) {
        self.current_query_hash = Some(query_hash);
        self.is_prepared = is_prepared;
        self.last_activity = Instant::now();
        self.query_count += 1;
    }

    /// Mark query end
    pub fn end_query(&mut self) {
        self.current_query_hash = None;
        self.is_prepared = false;
        self.prepared_stmt_name = None;
        self.last_activity = Instant::now();
    }

    /// Session idle time
    pub fn idle_time(&self) -> Duration {
        self.last_activity.elapsed()
    }

    /// Session age
    pub fn age(&self) -> Duration {
        self.session_start.elapsed()
    }
}

/// Prepared statement information
#[derive(Debug, Clone)]
pub struct PreparedStatement {
    /// Statement name
    pub name: String,
    /// Query hash of the underlying SQL
    pub query_hash: u64,
    /// Original query text (truncated)
    pub query_text: String,
    /// Creation time
    pub created_at: Instant,
    /// Number of times executed
    pub execution_count: u64,
    /// Parameter types (if known)
    pub param_types: Vec<String>,
}

impl PreparedStatement {
    pub fn new(name: String, query_hash: u64, query_text: String) -> Self {
        Self {
            name,
            query_hash,
            query_text,
            created_at: Instant::now(),
            execution_count: 0,
            param_types: Vec::new(),
        }
    }
}

/// Portal (bound prepared statement ready for execution)
#[derive(Debug, Clone)]
pub struct Portal {
    /// Portal name
    pub name: String,
    /// Associated prepared statement name
    pub statement_name: String,
    /// Query hash
    pub query_hash: u64,
    /// Bind time
    pub bound_at: Instant,
    /// Maximum rows to return (0 = unlimited)
    pub max_rows: i64,
}

/// Session tracker for all active PostgreSQL backends
pub struct SessionTracker {
    /// Active sessions by PID
    sessions: Arc<RwLock<HashMap<u32, SessionState>>>,
    /// Prepared statements by (pid, statement_name)
    prepared_stmts: Arc<RwLock<HashMap<(u32, String), PreparedStatement>>>,
    /// Portals by (pid, portal_name)
    portals: Arc<RwLock<HashMap<(u32, String), Portal>>>,
    /// Maximum session idle time before cleanup
    max_idle_time: Duration,
    /// Statistics
    stats: Arc<RwLock<TrackerStats>>,
}

#[derive(Debug, Default, Clone)]
pub struct TrackerStats {
    pub active_sessions: usize,
    pub total_sessions_tracked: u64,
    pub prepared_stmts_count: usize,
    pub portals_count: usize,
    pub simple_queries: u64,
    pub prepared_queries: u64,
    pub sessions_cleaned: u64,
}

impl SessionTracker {
    /// Create a new session tracker
    pub fn new() -> Self {
        Self::with_idle_timeout(Duration::from_secs(300)) // 5 minutes default
    }

    /// Create with custom idle timeout
    pub fn with_idle_timeout(max_idle_time: Duration) -> Self {
        Self {
            sessions: Arc::new(RwLock::new(HashMap::new())),
            prepared_stmts: Arc::new(RwLock::new(HashMap::new())),
            portals: Arc::new(RwLock::new(HashMap::new())),
            max_idle_time,
            stats: Arc::new(RwLock::new(TrackerStats::default())),
        }
    }

    /// Get or create a session for a PID
    pub async fn get_or_create_session(&self, pid: u32) -> SessionState {
        let mut sessions = self.sessions.write().await;
        
        if let Some(session) = sessions.get(&pid) {
            return session.clone();
        }

        let session = SessionState::new(pid);
        sessions.insert(pid, session.clone());

        let mut stats = self.stats.write().await;
        stats.total_sessions_tracked += 1;
        stats.active_sessions = sessions.len();

        session
    }

    /// Record a simple query start (direct SQL execution)
    pub async fn record_simple_query_start(&self, pid: u32, query_hash: u64) {
        let mut sessions = self.sessions.write().await;
        
        let session = sessions.entry(pid).or_insert_with(|| SessionState::new(pid));
        session.start_query(query_hash, false);

        let mut stats = self.stats.write().await;
        stats.simple_queries += 1;

        trace!(pid = pid, hash = query_hash, "Simple query started");
    }

    /// Record a query end
    pub async fn record_query_end(&self, pid: u32) {
        let mut sessions = self.sessions.write().await;
        
        if let Some(session) = sessions.get_mut(&pid) {
            session.end_query();
            trace!(pid = pid, "Query ended");
        }
    }

    /// Record a Parse message (prepared statement creation)
    pub async fn record_parse(&self, pid: u32, stmt_name: String, query_hash: u64, query_text: String) {
        let stmt = PreparedStatement::new(stmt_name.clone(), query_hash, query_text);
        
        let mut stmts = self.prepared_stmts.write().await;
        stmts.insert((pid, stmt_name.clone()), stmt);

        let mut stats = self.stats.write().await;
        stats.prepared_stmts_count = stmts.len();

        debug!(pid = pid, stmt = %stmt_name, hash = query_hash, "Prepared statement created");
    }

    /// Record a Bind message (portal creation)
    pub async fn record_bind(
        &self,
        pid: u32,
        portal_name: String,
        stmt_name: String,
        max_rows: i64,
    ) {
        // Look up the prepared statement to get the query hash
        let stmts = self.prepared_stmts.read().await;
        let query_hash = stmts
            .get(&(pid, stmt_name.clone()))
            .map(|s| s.query_hash)
            .unwrap_or(0);
        drop(stmts);

        let portal = Portal {
            name: portal_name.clone(),
            statement_name: stmt_name.clone(),
            query_hash,
            bound_at: Instant::now(),
            max_rows,
        };

        let mut portals = self.portals.write().await;
        portals.insert((pid, portal_name.clone()), portal);

        let mut stats = self.stats.write().await;
        stats.portals_count = portals.len();

        trace!(pid = pid, portal = %portal_name, stmt = %stmt_name, "Portal bound");
    }

    /// Record an Execute message (prepared statement execution)
    pub async fn record_execute(&self, pid: u32, portal_name: String) {
        // Look up the portal to get query info
        let portals = self.portals.read().await;
        let query_hash = portals
            .get(&(pid, portal_name.clone()))
            .map(|p| p.query_hash)
            .unwrap_or(0);
        let stmt_name = portals
            .get(&(pid, portal_name.clone()))
            .map(|p| p.statement_name.clone());
        drop(portals);

        // Update session
        let mut sessions = self.sessions.write().await;
        let session = sessions.entry(pid).or_insert_with(|| SessionState::new(pid));
        session.start_query(query_hash, true);
        session.prepared_stmt_name = stmt_name.clone();

        // Increment prepared statement execution count
        if let Some(stmt_name) = stmt_name {
            let mut stmts = self.prepared_stmts.write().await;
            if let Some(stmt) = stmts.get_mut(&(pid, stmt_name)) {
                stmt.execution_count += 1;
            }
        }

        let mut stats = self.stats.write().await;
        stats.prepared_queries += 1;

        trace!(pid = pid, portal = %portal_name, hash = query_hash, "Prepared statement executed");
    }

    /// Record a Close message (portal/statement destruction)
    pub async fn record_close(&self, pid: u32, close_type: CloseType, name: String) {
        match close_type {
            CloseType::Statement => {
                let mut stmts = self.prepared_stmts.write().await;
                stmts.remove(&(pid, name.clone()));
                
                let mut stats = self.stats.write().await;
                stats.prepared_stmts_count = stmts.len();
                
                trace!(pid = pid, stmt = %name, "Prepared statement closed");
            }
            CloseType::Portal => {
                let mut portals = self.portals.write().await;
                portals.remove(&(pid, name.clone()));
                
                let mut stats = self.stats.write().await;
                stats.portals_count = portals.len();
                
                trace!(pid = pid, portal = %name, "Portal closed");
            }
        }
    }

    /// Update session metadata from pg_stat_activity
    pub async fn update_session_metadata(
        &self,
        pid: u32,
        application_name: Option<String>,
        client_addr: Option<String>,
        database: Option<String>,
        username: Option<String>,
    ) {
        let mut sessions = self.sessions.write().await;
        
        if let Some(session) = sessions.get_mut(&pid) {
            session.application_name = application_name;
            session.client_addr = client_addr;
            session.database = database;
            session.username = username;
        }
    }

    /// Get current query hash for a PID
    pub async fn get_current_query_hash(&self, pid: u32) -> Option<u64> {
        let sessions = self.sessions.read().await;
        sessions.get(&pid).and_then(|s| s.current_query_hash)
    }

    /// Get session info for a PID
    pub async fn get_session(&self, pid: u32) -> Option<SessionState> {
        let sessions = self.sessions.read().await;
        sessions.get(&pid).cloned()
    }

    /// Get all active sessions
    pub async fn get_all_sessions(&self) -> Vec<SessionState> {
        let sessions = self.sessions.read().await;
        sessions.values().cloned().collect()
    }

    /// Get prepared statement info
    pub async fn get_prepared_statement(&self, pid: u32, name: &str) -> Option<PreparedStatement> {
        let stmts = self.prepared_stmts.read().await;
        stmts.get(&(pid, name.to_string())).cloned()
    }

    /// Cleanup idle sessions
    pub async fn cleanup_idle_sessions(&self) -> usize {
        let mut sessions = self.sessions.write().await;
        let mut stmts = self.prepared_stmts.write().await;
        let mut portals = self.portals.write().await;

        let pids_to_remove: Vec<u32> = sessions
            .iter()
            .filter(|(_, s)| s.idle_time() > self.max_idle_time)
            .map(|(pid, _)| *pid)
            .collect();

        let count = pids_to_remove.len();

        for pid in &pids_to_remove {
            sessions.remove(pid);
            
            // Remove associated prepared statements
            stmts.retain(|(p, _), _| p != pid);
            
            // Remove associated portals
            portals.retain(|(p, _), _| p != pid);
        }

        if count > 0 {
            let mut stats = self.stats.write().await;
            stats.sessions_cleaned += count as u64;
            stats.active_sessions = sessions.len();
            stats.prepared_stmts_count = stmts.len();
            stats.portals_count = portals.len();
            
            debug!("Cleaned up {} idle sessions", count);
        }

        count
    }

    /// Remove a session explicitly (e.g., backend terminated)
    pub async fn remove_session(&self, pid: u32) {
        let mut sessions = self.sessions.write().await;
        let mut stmts = self.prepared_stmts.write().await;
        let mut portals = self.portals.write().await;

        sessions.remove(&pid);
        stmts.retain(|(p, _), _| *p != pid);
        portals.retain(|(p, _), _| *p != pid);

        let mut stats = self.stats.write().await;
        stats.active_sessions = sessions.len();
        stats.prepared_stmts_count = stmts.len();
        stats.portals_count = portals.len();
    }

    /// Get tracker statistics
    pub async fn get_stats(&self) -> TrackerStats {
        self.stats.read().await.clone()
    }
}

impl Default for SessionTracker {
    fn default() -> Self {
        Self::new()
    }
}

/// Type of close operation
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseType {
    Statement,
    Portal,
}

/// Query execution context with full session info
#[derive(Debug, Clone)]
pub struct QueryContext {
    pub pid: u32,
    pub query_hash: u64,
    pub is_prepared: bool,
    pub prepared_stmt_name: Option<String>,
    pub application_name: Option<String>,
    pub database: Option<String>,
    pub username: Option<String>,
    pub session_query_count: u64,
}

impl QueryContext {
    /// Build context from session state
    pub fn from_session(session: &SessionState, query_hash: u64) -> Self {
        Self {
            pid: session.backend_pid,
            query_hash,
            is_prepared: session.is_prepared,
            prepared_stmt_name: session.prepared_stmt_name.clone(),
            application_name: session.application_name.clone(),
            database: session.database.clone(),
            username: session.username.clone(),
            session_query_count: session.query_count,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_simple_query_tracking() {
        let tracker = SessionTracker::new();

        tracker.record_simple_query_start(1000, 12345).await;

        let session = tracker.get_session(1000).await.unwrap();
        assert_eq!(session.current_query_hash, Some(12345));
        assert!(!session.is_prepared);
        assert_eq!(session.query_count, 1);

        tracker.record_query_end(1000).await;

        let session = tracker.get_session(1000).await.unwrap();
        assert_eq!(session.current_query_hash, None);
    }

    #[tokio::test]
    async fn test_prepared_statement_flow() {
        let tracker = SessionTracker::new();

        // Parse: create prepared statement
        tracker
            .record_parse(1000, "stmt1".to_string(), 12345, "SELECT * FROM users".to_string())
            .await;

        // Bind: create portal
        tracker
            .record_bind(1000, "portal1".to_string(), "stmt1".to_string(), 0)
            .await;

        // Execute: run the portal
        tracker.record_execute(1000, "portal1".to_string()).await;

        let session = tracker.get_session(1000).await.unwrap();
        assert_eq!(session.current_query_hash, Some(12345));
        assert!(session.is_prepared);
        assert_eq!(session.prepared_stmt_name, Some("stmt1".to_string()));

        let stats = tracker.get_stats().await;
        assert_eq!(stats.prepared_queries, 1);
        assert_eq!(stats.prepared_stmts_count, 1);
        assert_eq!(stats.portals_count, 1);
    }

    #[tokio::test]
    async fn test_session_cleanup() {
        let tracker = SessionTracker::with_idle_timeout(Duration::from_millis(10));

        tracker.record_simple_query_start(1000, 12345).await;
        tracker.record_query_end(1000).await;

        // Wait for idle timeout
        tokio::time::sleep(Duration::from_millis(20)).await;

        let cleaned = tracker.cleanup_idle_sessions().await;
        assert_eq!(cleaned, 1);

        let session = tracker.get_session(1000).await;
        assert!(session.is_none());
    }

    #[tokio::test]
    async fn test_close_operations() {
        let tracker = SessionTracker::new();

        // Create statement and portal
        tracker
            .record_parse(1000, "stmt1".to_string(), 12345, "SELECT 1".to_string())
            .await;
        tracker
            .record_bind(1000, "portal1".to_string(), "stmt1".to_string(), 0)
            .await;

        let stats = tracker.get_stats().await;
        assert_eq!(stats.prepared_stmts_count, 1);
        assert_eq!(stats.portals_count, 1);

        // Close portal
        tracker
            .record_close(1000, CloseType::Portal, "portal1".to_string())
            .await;

        let stats = tracker.get_stats().await;
        assert_eq!(stats.portals_count, 0);

        // Close statement
        tracker
            .record_close(1000, CloseType::Statement, "stmt1".to_string())
            .await;

        let stats = tracker.get_stats().await;
        assert_eq!(stats.prepared_stmts_count, 0);
    }

    #[tokio::test]
    async fn test_session_metadata() {
        let tracker = SessionTracker::new();

        tracker.record_simple_query_start(1000, 12345).await;

        tracker
            .update_session_metadata(
                1000,
                Some("myapp".to_string()),
                Some("127.0.0.1".to_string()),
                Some("testdb".to_string()),
                Some("testuser".to_string()),
            )
            .await;

        let session = tracker.get_session(1000).await.unwrap();
        assert_eq!(session.application_name, Some("myapp".to_string()));
        assert_eq!(session.database, Some("testdb".to_string()));
    }
}
