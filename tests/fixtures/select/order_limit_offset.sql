-- @setup
CREATE TABLE products (id INT, name TEXT, price FLOAT);
INSERT INTO products VALUES (1, 'Widget', 19.99), (2, 'Gadget', 29.99), (3, 'Doohickey', 9.99), (4, 'Thingamajig', 39.99);

-- @test
SELECT name, price FROM products ORDER BY price DESC LIMIT 2 OFFSET 1;
