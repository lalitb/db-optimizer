-- Scenario 3: Join Path Issue - Missing Index on Foreign Key
-- 
-- Classic N+1 problem: orders joined to customers without FK index
-- Expected: Vaanyze recommends CREATE INDEX ON orders(customer_id)
--
-- This tests join optimization detection

-- Drop if exists
DROP TABLE IF EXISTS order_items CASCADE;
DROP TABLE IF EXISTS orders CASCADE;
DROP TABLE IF EXISTS customers CASCADE;

-- Create customers table
CREATE TABLE customers (
    id SERIAL PRIMARY KEY,
    email VARCHAR(255) NOT NULL,
    name VARCHAR(200) NOT NULL,
    tier VARCHAR(20) DEFAULT 'standard',
    created_at TIMESTAMP DEFAULT NOW()
);

-- Create orders table (NO INDEX on customer_id - intentional)
CREATE TABLE orders (
    id SERIAL PRIMARY KEY,
    customer_id INTEGER NOT NULL,  -- FK but no index!
    order_date TIMESTAMP NOT NULL,
    status VARCHAR(20) DEFAULT 'pending',
    total_amount DECIMAL(10,2) NOT NULL
);

-- Generate deterministic customers (10K)
INSERT INTO customers (email, name, tier, created_at)
SELECT 
    'customer' || i || '@example.com',
    'Customer ' || i,
    CASE (i % 4)
        WHEN 0 THEN 'premium'
        WHEN 1 THEN 'standard'
        WHEN 2 THEN 'standard'
        ELSE 'basic'
    END,
    '2023-01-01'::timestamp + (i || ' hours')::interval
FROM generate_series(1, 10000) i;

-- Generate deterministic orders (200K) - ~20 orders per customer avg
INSERT INTO orders (customer_id, order_date, status, total_amount)
SELECT 
    ((i - 1) % 10000) + 1 as customer_id,  -- Distribute across customers
    '2024-01-01'::timestamp + (i || ' minutes')::interval,
    CASE (i % 4)
        WHEN 0 THEN 'completed'
        WHEN 1 THEN 'completed'
        WHEN 2 THEN 'shipped'
        ELSE 'pending'
    END,
    (50 + (i % 1000))::decimal(10,2)
FROM generate_series(1, 200000) i;

-- Analyze
ANALYZE customers;
ANALYZE orders;

-- Verify
SELECT 'customers' as table_name, COUNT(*) as row_count FROM customers
UNION ALL
SELECT 'orders', COUNT(*) FROM orders;
