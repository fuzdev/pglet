-- @setup
CREATE TABLE products (id INT, name TEXT, price INT);
INSERT INTO products VALUES (1, 'Apple', 100), (2, 'Banana', 50), (3, 'Cherry', 200);

-- @test
SELECT * FROM (SELECT id, name FROM products WHERE price > 60) AS expensive ORDER BY id;
