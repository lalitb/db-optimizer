-- Scenario 6: WAL/Fsync Pressure - Negative Test
-- 
-- Workload that creates I/O pressure but should NOT result in index recommendation
-- Expected: Vaanyze does NOT recommend random indexes
--
-- This tests that Vaanyze doesn't produce false positives

-- Drop if exists
DROP TABLE IF EXISTS audit_log CASCADE;

-- Create audit log table (append-only, rarely queried)
CREATE TABLE audit_log (
    id SERIAL PRIMARY KEY,
    action VARCHAR(50) NOT NULL,
    entity_type VARCHAR(50) NOT NULL,
    entity_id INTEGER NOT NULL,
    user_id INTEGER,
    details JSONB DEFAULT '{}',
    created_at TIMESTAMP DEFAULT NOW()
);

-- Generate initial data (200K rows)
INSERT INTO audit_log (action, entity_type, entity_id, user_id, details, created_at)
SELECT 
    CASE (i % 6)
        WHEN 0 THEN 'create'
        WHEN 1 THEN 'update'
        WHEN 2 THEN 'delete'
        WHEN 3 THEN 'view'
        WHEN 4 THEN 'export'
        ELSE 'login'
    END,
    CASE (i % 4)
        WHEN 0 THEN 'user'
        WHEN 1 THEN 'order'
        WHEN 2 THEN 'product'
        ELSE 'report'
    END,
    (i % 10000) + 1,
    (i % 500) + 1,
    jsonb_build_object('ip', '192.168.1.' || (i % 255), 'browser', 'Chrome'),
    '2024-01-01'::timestamp + (i || ' seconds')::interval
FROM generate_series(1, 200000) i;

-- Analyze
ANALYZE audit_log;

-- Verify
SELECT 'audit_log' as table_name, COUNT(*) as row_count FROM audit_log;
