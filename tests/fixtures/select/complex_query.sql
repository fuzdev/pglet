-- @setup
CREATE TABLE orders (id INT, customer TEXT, amount FLOAT, status TEXT);
INSERT INTO orders VALUES
  (1, 'Alice', 100.50, 'pending'),
  (2, 'Bob', 250.00, 'completed'),
  (3, 'Alice', 75.25, 'completed'),
  (4, 'Carol', 300.00, 'pending'),
  (5, 'Bob', 125.75, 'completed');

-- @test
SELECT customer, amount
FROM orders
WHERE status = 'completed' AND amount > 100
ORDER BY amount DESC
LIMIT 2;
