-- Scenario 1: Missing Index on WHERE Predicate
-- 
-- A simple equality filter on a non-indexed column.
-- Expected: Vaanyze recommends CREATE INDEX on users.email
--
-- Schema: users table with email column frequently filtered

-- Drop if exists for idempotency
DROP TABLE IF EXISTS users CASCADE;

-- Create table
CREATE TABLE users (
    id SERIAL PRIMARY KEY,
    email VARCHAR(255) NOT NULL,
    username VARCHAR(100) NOT NULL,
    status VARCHAR(20) DEFAULT 'active',
    created_at TIMESTAMP DEFAULT NOW(),
    updated_at TIMESTAMP DEFAULT NOW()
);

-- Generate deterministic data (fixed seed via generate_series)
-- 100K users with predictable emails
INSERT INTO users (email, username, status, created_at)
SELECT 
    'user' || i || '@example.com',
    'user_' || i,
    CASE (i % 5) 
        WHEN 0 THEN 'active'
        WHEN 1 THEN 'active'
        WHEN 2 THEN 'active'
        WHEN 3 THEN 'inactive'
        ELSE 'pending'
    END,
    '2024-01-01'::timestamp + (i || ' seconds')::interval
FROM generate_series(1, 100000) i;

-- Analyze for accurate stats
ANALYZE users;

-- Verify data loaded
SELECT 'users' as table_name, COUNT(*) as row_count FROM users;
