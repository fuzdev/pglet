-- @setup
CREATE TABLE products (id INT, name TEXT);
INSERT INTO products VALUES (1, 'Apple'), (2, 'Banana'), (3, 'Apricot'), (4, 'Orange');

-- @test
SELECT * FROM products WHERE name LIKE 'A%' ORDER BY id;
