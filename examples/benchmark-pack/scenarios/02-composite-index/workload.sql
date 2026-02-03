-- Scenario 2: Composite Index - Load Queries
-- Multi-tenant queries that need (tenant_id, created_at) index

-- Query 1: Tenant dashboard - recent events (PRIMARY target)
SELECT id, event_type, created_at, payload
FROM events
WHERE tenant_id = 42
  AND created_at >= '2024-02-01'
  AND created_at < '2024-03-01'
ORDER BY created_at DESC
LIMIT 100;

-- Query 2: Different tenant, different date range
SELECT id, event_type, created_at, payload
FROM events
WHERE tenant_id = 17
  AND created_at >= '2024-01-15'
  AND created_at < '2024-02-15'
ORDER BY created_at DESC
LIMIT 100;

-- Query 3: Tenant event counts by type in time range
SELECT event_type, COUNT(*) as count
FROM events
WHERE tenant_id = 55
  AND created_at >= '2024-01-01'
  AND created_at < '2024-02-01'
GROUP BY event_type
ORDER BY count DESC;

-- Query 4: Multiple tenants (should still benefit)
SELECT tenant_id, COUNT(*) as event_count
FROM events
WHERE tenant_id IN (1, 2, 3, 4, 5)
  AND created_at >= '2024-02-01'
GROUP BY tenant_id
ORDER BY event_count DESC;

-- Query 5: Single tenant, all time (partial benefit)
SELECT COUNT(*) as total_events
FROM events
WHERE tenant_id = 88;
