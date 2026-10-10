-- @setup
CREATE TABLE cast_test2 (id INT, price FLOAT);
INSERT INTO cast_test2 VALUES (1, 19.99);
INSERT INTO cast_test2 VALUES (2, 9.50);

-- @test
-- PostgreSQL double-colon cast syntax
SELECT id::text AS id_str, price::int AS price_int FROM cast_test2 ORDER BY id;
