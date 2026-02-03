-- Scenario 5: Write-Heavy Workload - Mixed Read/Write
-- This workload has heavy writes that will be impacted by index

-- WRITE: Insert new metric (70% of workload)
INSERT INTO metrics (sensor_id, metric_name, metric_value, recorded_at)
VALUES (
    (random() * 999 + 1)::integer,
    CASE (random() * 4)::integer
        WHEN 0 THEN 'temperature'
        WHEN 1 THEN 'humidity'
        WHEN 2 THEN 'pressure'
        WHEN 3 THEN 'voltage'
        ELSE 'current'
    END,
    (20.0 + random() * 80)::decimal(15,4),
    NOW()
);

-- READ: Query that would benefit from index (30% of workload)
SELECT sensor_id, metric_name, AVG(metric_value) as avg_value, COUNT(*) as count
FROM metrics
WHERE sensor_id = 500
  AND recorded_at >= NOW() - INTERVAL '1 hour'
GROUP BY sensor_id, metric_name;
