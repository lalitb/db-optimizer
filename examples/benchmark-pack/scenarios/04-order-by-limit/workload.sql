-- Scenario 4: ORDER BY + LIMIT - Load Queries
-- Pagination and sorting queries that need index

-- Query 1: Cheapest products (PRIMARY target)
SELECT id, name, category, price, rating
FROM products
ORDER BY price ASC
LIMIT 20;

-- Query 2: Most expensive products
SELECT id, name, category, price, rating
FROM products
ORDER BY price DESC
LIMIT 20;

-- Query 3: Category page with price sorting (needs composite index)
SELECT id, name, price, rating, stock
FROM products
WHERE category = 'Electronics'
ORDER BY price ASC
LIMIT 20;

-- Query 4: Different category
SELECT id, name, price, rating, stock
FROM products
WHERE category = 'Clothing'
ORDER BY price DESC
LIMIT 20;

-- Query 5: Top rated in category
SELECT id, name, price, rating
FROM products
WHERE category = 'Books'
ORDER BY rating DESC, price ASC
LIMIT 20;

-- Query 6: Paginated results (offset pattern)
SELECT id, name, category, price
FROM products
ORDER BY price ASC
LIMIT 20 OFFSET 100;

-- Query 7: In-stock products sorted by price
SELECT id, name, price, stock
FROM products
WHERE stock > 0
ORDER BY price ASC
LIMIT 50;
