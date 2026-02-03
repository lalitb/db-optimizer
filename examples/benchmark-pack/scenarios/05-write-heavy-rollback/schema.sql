-- Scenario 5: Write-Heavy Workload - Rollback Test
-- 
-- High write rate table where adding an index causes write regression
-- Expected: Vaanyze applies index, detects regression, triggers rollback
--
-- This tests the rollback mechanism

-- Drop if exists
DROP TABLE IF EXISTS metrics CASCADE;

-- Create high-write metrics table
CREATE TABLE metrics (
    id SERIAL PRIMARY KEY,
    sensor_id INTEGER NOT NULL,
    metric_name VARCHAR(100) NOT NULL,
    metric_value DECIMAL(15,4) NOT NULL,
    recorded_at TIMESTAMP DEFAULT NOW()
);

-- Generate initial deterministic data (100K rows)
INSERT INTO metrics (sensor_id, metric_name, metric_value, recorded_at)
SELECT 
    (i % 1000) + 1 as sensor_id,  -- 1000 sensors
    CASE (i % 5)
        WHEN 0 THEN 'temperature'
        WHEN 1 THEN 'humidity'
        WHEN 2 THEN 'pressure'
        WHEN 3 THEN 'voltage'
        ELSE 'current'
    END,
    (20.0 + (i % 100))::decimal(15,4),
    '2024-01-01'::timestamp + (i || ' seconds')::interval
FROM generate_series(1, 100000) i;

-- Analyze
ANALYZE metrics;

-- Verify
SELECT 'metrics' as table_name, COUNT(*) as row_count FROM metrics;
