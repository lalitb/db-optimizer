-- Vaanyze Benchmark Pack - PostgreSQL Initialization
-- Creates pg_stat_statements extension and sets up base schema

-- Enable required extensions
CREATE EXTENSION IF NOT EXISTS pg_stat_statements;

-- Create benchmark metadata table
CREATE TABLE IF NOT EXISTS _benchmark_meta (
    id SERIAL PRIMARY KEY,
    scenario_name VARCHAR(100) NOT NULL,
    started_at TIMESTAMP DEFAULT NOW(),
    completed_at TIMESTAMP,
    status VARCHAR(20) DEFAULT 'running',
    baseline_captured BOOLEAN DEFAULT FALSE,
    recommendation_id VARCHAR(100),
    index_applied BOOLEAN DEFAULT FALSE,
    rollback_triggered BOOLEAN DEFAULT FALSE
);

-- Grant permissions
GRANT ALL PRIVILEGES ON ALL TABLES IN SCHEMA public TO postgres;
GRANT USAGE, SELECT ON ALL SEQUENCES IN SCHEMA public TO postgres;

-- Reset pg_stat_statements for clean benchmark
SELECT pg_stat_statements_reset();

-- Log initialization
DO $$
BEGIN
    RAISE NOTICE 'Vaanyze Benchmark Pack - PostgreSQL initialized';
    RAISE NOTICE 'pg_stat_statements enabled and reset';
END $$;
