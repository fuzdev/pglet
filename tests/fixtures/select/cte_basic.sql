-- @setup
CREATE TABLE products (id INT, name TEXT, price INT);
INSERT INTO products VALUES (1, 'Apple', 100), (2, 'Banana', 50), (3, 'Cherry', 200);

-- @test
WITH expensive AS (
    SELECT id, name, price FROM products WHERE price > 60
)
SELECT * FROM expensive ORDER BY id;
