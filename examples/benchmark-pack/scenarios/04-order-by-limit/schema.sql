-- Scenario 4: ORDER BY + LIMIT Without Index
-- 
-- Pagination query with sorting on non-indexed column
-- Expected: Vaanyze recommends CREATE INDEX ON products(price DESC)
-- or CREATE INDEX ON products(category, price)
--
-- This tests ORDER BY optimization detection

-- Drop if exists
DROP TABLE IF EXISTS products CASCADE;

-- Create products table
CREATE TABLE products (
    id SERIAL PRIMARY KEY,
    name VARCHAR(255) NOT NULL,
    category VARCHAR(100) NOT NULL,
    price DECIMAL(10,2) NOT NULL,
    stock INTEGER DEFAULT 0,
    rating DECIMAL(3,2) DEFAULT 0.00,
    created_at TIMESTAMP DEFAULT NOW()
);

-- Generate deterministic products (50K)
INSERT INTO products (name, category, price, stock, rating, created_at)
SELECT 
    'Product ' || i,
    CASE (i % 10)
        WHEN 0 THEN 'Electronics'
        WHEN 1 THEN 'Clothing'
        WHEN 2 THEN 'Books'
        WHEN 3 THEN 'Home'
        WHEN 4 THEN 'Sports'
        WHEN 5 THEN 'Toys'
        WHEN 6 THEN 'Food'
        WHEN 7 THEN 'Beauty'
        WHEN 8 THEN 'Auto'
        ELSE 'Other'
    END,
    (10 + (i % 1000))::decimal(10,2),  -- Price: $10 - $1009
    (i % 500),  -- Stock: 0-499
    (1.0 + ((i % 40)::decimal / 10.0)),  -- Rating: 1.0 - 5.0
    '2024-01-01'::timestamp + (i || ' minutes')::interval
FROM generate_series(1, 50000) i;

-- Analyze
ANALYZE products;

-- Verify
SELECT 'products' as table_name, COUNT(*) as row_count FROM products;
SELECT category, COUNT(*) as count FROM products GROUP BY category ORDER BY count DESC;
