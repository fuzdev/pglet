-- A function with a `text` argument types the parameter there.
-- @setup
CREATE TABLE up_t (id INT, name TEXT, n BIGINT);
INSERT INTO up_t VALUES (1, 'x', 10), (2, 'y', 20), (3, NULL, 30);
-- @param $1 text x
-- @test
SELECT id FROM up_t WHERE upper($1) = upper(name) OR $1 IS NULL ORDER BY id;
