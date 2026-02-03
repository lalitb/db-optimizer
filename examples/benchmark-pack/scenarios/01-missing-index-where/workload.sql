-- Scenario 1: Missing Index on WHERE Predicate - Load Queries
-- Run these queries repeatedly to simulate workload

-- Query 1: Email lookup (PRIMARY target for index recommendation)
-- Should be slow without index, fast with index
SELECT id, email, username, status, created_at
FROM users
WHERE email = 'user50000@example.com';

-- Query 2: Variation with different email
SELECT id, email, username, status, created_at
FROM users
WHERE email = 'user25000@example.com';

-- Query 3: Another variation
SELECT id, email, username, status, created_at
FROM users
WHERE email = 'user75000@example.com';

-- Query 4: Range query on email (should also benefit)
SELECT id, email, username, status
FROM users
WHERE email LIKE 'user100%'
ORDER BY email
LIMIT 100;

-- Query 5: Count by email domain (less likely to benefit)
SELECT COUNT(*) as user_count
FROM users
WHERE email LIKE '%@example.com';
