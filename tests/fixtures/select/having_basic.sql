-- @setup
CREATE TABLE products (id INT, category TEXT, price FLOAT);
INSERT INTO products VALUES
  (1, 'electronics', 100.0),
  (2, 'electronics', 200.0),
  (3, 'electronics', 150.0),
  (4, 'clothing', 50.0),
  (5, 'clothing', 75.0),
  (6, 'food', 10.0);

-- @test
SELECT category, COUNT(*) FROM products GROUP BY category HAVING COUNT(*) > 2 ORDER BY category;
