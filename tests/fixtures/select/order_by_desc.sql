-- @setup
CREATE TABLE products (id INT, price FLOAT);
INSERT INTO products VALUES (1, 19.99), (2, 9.99), (3, 29.99);

-- @test
SELECT * FROM products ORDER BY price DESC;
