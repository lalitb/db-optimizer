-- Scenario 3: Join Path - Load Queries
-- Queries that join orders to customers (need FK index)

-- Query 1: Customer order history (PRIMARY target)
SELECT c.id, c.email, c.name, o.id as order_id, o.order_date, o.total_amount
FROM customers c
JOIN orders o ON o.customer_id = c.id
WHERE c.id = 5000
ORDER BY o.order_date DESC
LIMIT 50;

-- Query 2: Different customer
SELECT c.id, c.email, c.name, o.id as order_id, o.order_date, o.total_amount
FROM customers c
JOIN orders o ON o.customer_id = c.id
WHERE c.id = 2500
ORDER BY o.order_date DESC
LIMIT 50;

-- Query 3: Customer with aggregation
SELECT c.id, c.email, c.name, c.tier,
       COUNT(o.id) as order_count,
       SUM(o.total_amount) as total_spent
FROM customers c
JOIN orders o ON o.customer_id = c.id
WHERE c.id = 7500
GROUP BY c.id;

-- Query 4: Premium customers with their orders
SELECT c.id, c.email, c.tier,
       COUNT(o.id) as order_count,
       AVG(o.total_amount) as avg_order
FROM customers c
JOIN orders o ON o.customer_id = c.id
WHERE c.tier = 'premium'
GROUP BY c.id
ORDER BY order_count DESC
LIMIT 100;

-- Query 5: Recent orders with customer info
SELECT o.id, o.order_date, o.total_amount, o.status,
       c.email, c.name
FROM orders o
JOIN customers c ON c.id = o.customer_id
WHERE o.order_date >= '2024-03-01'
ORDER BY o.order_date DESC
LIMIT 100;
