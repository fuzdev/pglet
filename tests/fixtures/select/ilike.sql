-- @setup
CREATE TABLE products (id INT, name TEXT);
INSERT INTO products VALUES (1, 'Apple'), (2, 'APRICOT'), (3, 'apple pie'), (4, 'Banana'), (5, NULL);

-- @test
SELECT * FROM products WHERE name ILIKE 'ap%' ORDER BY id;
