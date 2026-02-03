-- Scenario 6: WAL/Fsync Pressure - Heavy Write Workload
-- Mostly writes, minimal reads - should NOT need indexes

-- WRITE: Append audit log entry (95% of workload)
INSERT INTO audit_log (action, entity_type, entity_id, user_id, details)
VALUES (
    CASE (random() * 5)::integer
        WHEN 0 THEN 'create'
        WHEN 1 THEN 'update'
        WHEN 2 THEN 'delete'
        WHEN 3 THEN 'view'
        WHEN 4 THEN 'export'
        ELSE 'login'
    END,
    CASE (random() * 3)::integer
        WHEN 0 THEN 'user'
        WHEN 1 THEN 'order'
        WHEN 2 THEN 'product'
        ELSE 'report'
    END,
    (random() * 10000 + 1)::integer,
    (random() * 500 + 1)::integer,
    jsonb_build_object('ip', '192.168.1.' || (random() * 254 + 1)::integer, 'ts', NOW())
);

-- READ: Rare full scan for compliance (5% of workload - not optimizable)
SELECT COUNT(*) as total_entries
FROM audit_log
WHERE created_at >= NOW() - INTERVAL '1 day';
