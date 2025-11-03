-- PostgreSQL Query Health Advisor - Test Query Workload
-- Complex queries designed to be SLOW without proper indexes

-- SLOW QUERY 1: Multi-table join for customer order history (NO INDEXES!)
-- Scans 500K customers + 1M orders + 3M order_items
SELECT 
    c.email,
    c.first_name,
    c.last_name,
    COUNT(DISTINCT o.id) as total_orders,
    COUNT(oi.id) as total_items,
    SUM(oi.price * oi.quantity) as lifetime_value
FROM customers c
LEFT JOIN orders o ON c.id = o.customer_id
LEFT JOIN order_items oi ON o.id = oi.order_id
WHERE c.country = 'USA'
GROUP BY c.id, c.email, c.first_name, c.last_name
HAVING COUNT(o.id) > 5
ORDER BY lifetime_value DESC
LIMIT 50;

-- SLOW QUERY 2: Product sales report (NO INDEXES!)
-- Scans 3M order_items + 10K products
SELECT 
    p.category,
    p.name,
    COUNT(oi.id) as times_sold,
    SUM(oi.quantity) as total_quantity,
    SUM(oi.price * oi.quantity) as total_revenue
FROM products p
JOIN order_items oi ON p.id = oi.product_id
GROUP BY p.id, p.category, p.name
HAVING SUM(oi.quantity) > 10
ORDER BY total_revenue DESC
LIMIT 100;

-- SLOW QUERY 3: Customer lookup by email (NO INDEX!)
-- Scans all 500K customers
SELECT 
    c.*,
    COUNT(o.id) as order_count
FROM customers c
LEFT JOIN orders o ON c.id = o.customer_id
WHERE c.email LIKE '%customer12345%'
GROUP BY c.id;

-- SLOW QUERY 4: Recent orders with customer details (NO INDEX on status!)
-- Scans 1M orders + 500K customers
SELECT 
    o.id,
    o.order_date,
    o.total_amount,
    o.status,
    c.email,
    c.first_name,
    c.last_name,
    COUNT(oi.id) as item_count
FROM orders o
JOIN customers c ON o.customer_id = c.id
JOIN order_items oi ON o.id = oi.order_id
WHERE o.status IN ('pending', 'processing')
  AND o.order_date > NOW() - INTERVAL '30 days'
GROUP BY o.id, c.email, c.first_name, c.last_name
ORDER BY o.order_date DESC
LIMIT 100;

-- SLOW QUERY 5: Cross-customer product recommendations (VERY SLOW!)
-- Self-join on order_items - 3M x 3M potential combinations
SELECT DISTINCT
    oi1.product_id as product_a,
    oi2.product_id as product_b,
    COUNT(*) as times_bought_together
FROM order_items oi1
JOIN order_items oi2 ON oi1.order_id = oi2.order_id AND oi1.product_id < oi2.product_id
WHERE oi1.product_id IN (SELECT id FROM products WHERE category = 'Electronics' LIMIT 100)
GROUP BY oi1.product_id, oi2.product_id
HAVING COUNT(*) > 2
ORDER BY times_bought_together DESC
LIMIT 50;

-- SLOW QUERY 6: Customer segments by order value (NO INDEXES!)
-- Scans 500K customers + 1M orders
SELECT 
    c.country,
    CASE 
        WHEN AVG(o.total_amount) > 500 THEN 'High Value'
        WHEN AVG(o.total_amount) > 200 THEN 'Medium Value'
        ELSE 'Low Value'
    END as segment,
    COUNT(DISTINCT c.id) as customer_count,
    AVG(o.total_amount) as avg_order_value
FROM customers c
JOIN orders o ON c.id = o.customer_id
GROUP BY c.country, segment
ORDER BY customer_count DESC;

-- SLOW QUERY 7: Order fulfillment report (NO INDEX on order_id FK!)
-- Scans 3M order_items + 1M orders
SELECT 
    o.status,
    COUNT(DISTINCT o.id) as order_count,
    COUNT(oi.id) as total_items,
    AVG(oi.quantity) as avg_items_per_line,
    SUM(oi.price * oi.quantity) as total_value
FROM orders o
JOIN order_items oi ON o.id = oi.order_id
WHERE o.order_date > NOW() - INTERVAL '90 days'
GROUP BY o.status
ORDER BY total_value DESC;

-- SLOW QUERY 8: Top customers by category preference (COMPLEX!)
-- Multiple joins, aggregations, subqueries
SELECT 
    c.email,
    p.category,
    COUNT(DISTINCT o.id) as orders_in_category,
    SUM(oi.quantity) as items_purchased,
    SUM(oi.price * oi.quantity) as amount_spent
FROM customers c
JOIN orders o ON c.id = o.customer_id
JOIN order_items oi ON o.id = oi.order_id  
JOIN products p ON oi.product_id = p.id
WHERE c.country IN ('USA', 'Canada')
GROUP BY c.email, p.category
HAVING COUNT(DISTINCT o.id) > 3
ORDER BY amount_spent DESC
LIMIT 200;

-- The unused index (never query by created_at)
-- idx_products_created_at will be detected as unused!
