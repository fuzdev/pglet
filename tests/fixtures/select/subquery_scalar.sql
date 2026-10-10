-- @setup
CREATE TABLE categories (id INT, name TEXT);
CREATE TABLE products (id INT, name TEXT, category_id INT, price FLOAT);
INSERT INTO categories VALUES (1, 'Electronics'), (2, 'Clothing');
INSERT INTO products VALUES (1, 'Phone', 1, 999.99), (2, 'Laptop', 1, 1499.99), (3, 'Shirt', 2, 29.99);

-- @test
SELECT name, (SELECT MAX(price) FROM products) AS max_price FROM categories ORDER BY id;
