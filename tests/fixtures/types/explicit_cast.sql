-- @setup
CREATE TABLE cast_test (id INT, name TEXT);
INSERT INTO cast_test VALUES (1, 'Alice');
INSERT INTO cast_test VALUES (2, 'Bob');

-- @test
-- Explicit CAST from INT to TEXT
SELECT CAST(id AS TEXT) AS id_text, name FROM cast_test ORDER BY id;
