-- @setup
CREATE TABLE orders (id INT, category TEXT, amount INT);
INSERT INTO orders VALUES (1, 'A', 100), (2, 'B', 200), (3, 'A', 150), (4, 'B', 50);

-- @test
SELECT category, SUM(amount) FROM orders GROUP BY category ORDER BY category;
