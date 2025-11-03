-- PostgreSQL Query Health Advisor - Test Data Generator
-- This creates a realistic e-commerce database with performance issues

-- Drop existing tables
DROP TABLE IF EXISTS order_items CASCADE;
DROP TABLE IF EXISTS orders CASCADE;
DROP TABLE IF EXISTS products CASCADE;
DROP TABLE IF EXISTS customers CASCADE;

-- Create customers table (500K customers)
CREATE TABLE customers (
    id SERIAL PRIMARY KEY,
    email VARCHAR(255) NOT NULL,
    first_name VARCHAR(100),
    last_name VARCHAR(100),
    phone VARCHAR(20),
    created_at TIMESTAMP DEFAULT NOW(),
    country VARCHAR(50),
    city VARCHAR(100)
);

-- Create products table (10K products)
CREATE TABLE products (
    id SERIAL PRIMARY KEY,
    name VARCHAR(255) NOT NULL,
    sku VARCHAR(50) UNIQUE NOT NULL,
    category VARCHAR(100),
    price DECIMAL(10,2),
    stock_quantity INTEGER,
    created_at TIMESTAMP DEFAULT NOW()
);

-- Create orders table (1M orders)
CREATE TABLE orders (
    id SERIAL PRIMARY KEY,
    customer_id INTEGER NOT NULL,
    order_date TIMESTAMP DEFAULT NOW(),
    status VARCHAR(50),
    total_amount DECIMAL(10,2),
    shipping_address TEXT
);

-- Create order_items table (3M items)
CREATE TABLE order_items (
    id SERIAL PRIMARY KEY,
    order_id INTEGER NOT NULL,
    product_id INTEGER NOT NULL,
    quantity INTEGER,
    price DECIMAL(10,2)
);

-- Insert customers (500K records)
INSERT INTO customers (email, first_name, last_name, phone, country, city)
SELECT 
    'customer' || i || '@example.com',
    'First' || i,
    'Last' || i,
    '+1-555-' || LPAD(i::TEXT, 7, '0'),
    CASE (i % 5)
        WHEN 0 THEN 'USA'
        WHEN 1 THEN 'Canada'
        WHEN 2 THEN 'UK'
        WHEN 3 THEN 'Germany'
        ELSE 'France'
    END,
    'City' || (i % 100)
FROM generate_series(1, 500000) i;

-- Insert products (10K records)
INSERT INTO products (name, sku, category, price, stock_quantity)
SELECT 
    'Product ' || i,
    'SKU-' || LPAD(i::TEXT, 8, '0'),
    CASE (i % 10)
        WHEN 0 THEN 'Electronics'
        WHEN 1 THEN 'Clothing'
        WHEN 2 THEN 'Books'
        WHEN 3 THEN 'Home & Garden'
        WHEN 4 THEN 'Sports'
        WHEN 5 THEN 'Toys'
        WHEN 6 THEN 'Food'
        WHEN 7 THEN 'Beauty'
        WHEN 8 THEN 'Automotive'
        ELSE 'Other'
    END,
    (random() * 1000 + 10)::DECIMAL(10,2),
    (random() * 1000)::INTEGER
FROM generate_series(1, 10000) i;

-- Insert orders (1M records)
INSERT INTO orders (customer_id, order_date, status, total_amount)
SELECT 
    (random() * 499999 + 1)::INTEGER,
    NOW() - (random() * INTERVAL '365 days'),
    CASE (random() * 4)::INTEGER
        WHEN 0 THEN 'pending'
        WHEN 1 THEN 'processing'
        WHEN 2 THEN 'shipped'
        ELSE 'delivered'
    END,
    (random() * 1000 + 10)::DECIMAL(10,2)
FROM generate_series(1, 1000000) i;

-- Insert order_items (3M records)
INSERT INTO order_items (order_id, product_id, quantity, price)
SELECT 
    (random() * 999999 + 1)::INTEGER,
    (random() * 9999 + 1)::INTEGER,
    (random() * 5 + 1)::INTEGER,
    (random() * 100 + 5)::DECIMAL(10,2)
FROM generate_series(1, 3000000) i;

-- Create some indexes (but intentionally leave some missing)
-- Only index customers by id (primary key - auto created)
-- Only index products by id and sku
-- Intentionally NO indexes on:
--   - customers.email (will be queried frequently)
--   - orders.customer_id (foreign key, no index!)
--   - orders.status (frequently filtered)
--   - order_items.order_id (foreign key, no index!)
--   - order_items.product_id (foreign key, no index!)

-- Create an UNUSED index to test unused index detection
CREATE INDEX idx_products_created_at ON products(created_at);

ANALYZE customers;
ANALYZE products;
ANALYZE orders;
ANALYZE order_items;

-- Success message
SELECT 'Test data created successfully!' as message,
       (SELECT COUNT(*) FROM customers) as customers,
       (SELECT COUNT(*) FROM products) as products,
       (SELECT COUNT(*) FROM orders) as orders,
       (SELECT COUNT(*) FROM order_items) as order_items;
