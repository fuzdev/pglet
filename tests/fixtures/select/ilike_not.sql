-- NOT ILIKE with a NULL row (NULL propagates — the row is filtered, not matched).
-- @setup
CREATE TABLE products (id INT, name TEXT);
INSERT INTO products VALUES (1, 'Apple'), (2, 'BANANA'), (3, 'apricot'), (4, NULL);

-- @test
SELECT * FROM products WHERE name NOT ILIKE '%an%' ORDER BY id;
