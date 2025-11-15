-- Database schema for db-optimizer persistence layer
-- Compatible with both SQLite and PostgreSQL

-- Query performance history
CREATE TABLE IF NOT EXISTS query_samples (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    captured_at TIMESTAMP NOT NULL,
    query_id BIGINT NOT NULL,
    query_text TEXT NOT NULL,
    mean_exec_time_ms REAL NOT NULL,
    p95_exec_time_ms REAL NOT NULL,
    p99_exec_time_ms REAL NOT NULL,
    calls_per_min REAL NOT NULL,
    total_calls BIGINT NOT NULL,
    
    -- eBPF metrics (optional)
    cpu_time_us BIGINT,
    io_wait_time_us BIGINT,
    page_cache_misses INTEGER,
    context_switches INTEGER,
    
    -- Indexes for common queries
    UNIQUE(query_id, captured_at)
);

CREATE INDEX IF NOT EXISTS idx_query_samples_query_id ON query_samples(query_id);
CREATE INDEX IF NOT EXISTS idx_query_samples_captured_at ON query_samples(captured_at);

-- Recommendations history
CREATE TABLE IF NOT EXISTS recommendations (
    id TEXT PRIMARY KEY,
    created_at TIMESTAMP NOT NULL,
    issue_type TEXT NOT NULL,
    severity TEXT NOT NULL,
    title TEXT NOT NULL,
    description TEXT NOT NULL,
    estimated_savings_usd REAL NOT NULL,
    status TEXT NOT NULL, -- PENDING, APPLIED, VERIFIED, ROLLED_BACK, REJECTED
    applied_at TIMESTAMP,
    rolled_back_at TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_recommendations_status ON recommendations(status);
CREATE INDEX IF NOT EXISTS idx_recommendations_created_at ON recommendations(created_at);
CREATE INDEX IF NOT EXISTS idx_recommendations_applied_at ON recommendations(applied_at);

-- Change events tracking recommendation lifecycle
CREATE TABLE IF NOT EXISTS change_events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    recommendation_id TEXT NOT NULL,
    event_type TEXT NOT NULL, -- PROPOSED, APPLIED, VERIFIED, ROLLED_BACK
    timestamp TIMESTAMP NOT NULL,
    metadata TEXT NOT NULL, -- JSON
    
    FOREIGN KEY (recommendation_id) REFERENCES recommendations(id)
);

CREATE INDEX IF NOT EXISTS idx_change_events_recommendation_id ON change_events(recommendation_id);
CREATE INDEX IF NOT EXISTS idx_change_events_timestamp ON change_events(timestamp);

-- Baseline snapshots (pre/post apply metrics)
CREATE TABLE IF NOT EXISTS baselines (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    recommendation_id TEXT NOT NULL,
    baseline_type TEXT NOT NULL, -- PRE_APPLY, POST_APPLY
    captured_at TIMESTAMP NOT NULL,
    metrics TEXT NOT NULL, -- JSON
    
    FOREIGN KEY (recommendation_id) REFERENCES recommendations(id)
);

CREATE INDEX IF NOT EXISTS idx_baselines_recommendation_id ON baselines(recommendation_id);
CREATE INDEX IF NOT EXISTS idx_baselines_baseline_type ON baselines(baseline_type);

-- Schema version tracking
CREATE TABLE IF NOT EXISTS schema_version (
    version INTEGER PRIMARY KEY,
    applied_at TIMESTAMP NOT NULL
);

-- Insert initial schema version
INSERT OR IGNORE INTO schema_version (version, applied_at) 
VALUES (1, CURRENT_TIMESTAMP);
