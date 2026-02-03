-- Scenario 2: Missing Composite Index (tenant_id, created_at)
-- 
-- Multi-tenant SaaS pattern: queries filter by tenant + time range
-- Expected: Vaanyze recommends CREATE INDEX ON events(tenant_id, created_at)
--
-- This tests composite index detection and column ordering

-- Drop if exists
DROP TABLE IF EXISTS events CASCADE;

-- Create table (multi-tenant event log)
CREATE TABLE events (
    id SERIAL PRIMARY KEY,
    tenant_id INTEGER NOT NULL,
    event_type VARCHAR(50) NOT NULL,
    created_at TIMESTAMP NOT NULL,
    payload JSONB DEFAULT '{}'
);

-- Generate deterministic data
-- 500K events across 100 tenants, spread over 90 days
INSERT INTO events (tenant_id, event_type, created_at, payload)
SELECT 
    (i % 100) + 1 as tenant_id,  -- 100 tenants (1-100)
    CASE (i % 5)
        WHEN 0 THEN 'page_view'
        WHEN 1 THEN 'click'
        WHEN 2 THEN 'purchase'
        WHEN 3 THEN 'signup'
        ELSE 'logout'
    END as event_type,
    '2024-01-01'::timestamp + ((i % (90 * 24 * 60)) || ' minutes')::interval as created_at,
    jsonb_build_object('event_num', i, 'source', 'benchmark')
FROM generate_series(1, 500000) i;

-- Analyze for accurate stats
ANALYZE events;

-- Verify data
SELECT 'events' as table_name, COUNT(*) as row_count FROM events;
SELECT COUNT(DISTINCT tenant_id) as tenant_count FROM events;
