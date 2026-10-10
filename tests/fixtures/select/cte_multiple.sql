-- @setup
CREATE TABLE products (id INT, name TEXT, price INT);
INSERT INTO products VALUES (1, 'Apple', 100), (2, 'Banana', 50), (3, 'Cherry', 200), (4, 'Date', 75);

-- @test
WITH
    base AS (SELECT id, name, price FROM products WHERE price >= 50),
    filtered AS (SELECT id, name FROM base WHERE price < 150)
SELECT * FROM filtered ORDER BY id;
