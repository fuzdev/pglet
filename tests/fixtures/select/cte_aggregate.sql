-- @setup
CREATE TABLE orders (id INT, customer TEXT, amount INT);
INSERT INTO orders VALUES (1, 'Alice', 100), (2, 'Bob', 50), (3, 'Alice', 200), (4, 'Bob', 75), (5, 'Carol', 150);

-- @test
WITH totals AS (
    SELECT customer, SUM(amount) AS total FROM orders GROUP BY customer
)
SELECT * FROM totals ORDER BY customer;
