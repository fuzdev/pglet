-- `COALESCE($1, name)` types the parameter from its sibling column; the null test
-- after it reads a typed parameter.
-- @setup
CREATE TABLE up_t (id INT, name TEXT, n BIGINT);
INSERT INTO up_t VALUES (1, 'x', 10), (2, 'y', 20), (3, NULL, 30);
-- @param $1 text NULL
-- @test
SELECT id FROM up_t WHERE name = COALESCE($1, name) AND $1 IS NULL ORDER BY id;
