-- The `UPDATE` spelling of the comparison-first optional filter.
-- @setup
CREATE TABLE up_t (id INT, name TEXT, n BIGINT);
INSERT INTO up_t VALUES (1, 'x', 10), (2, 'y', 20), (3, NULL, 30);
-- @param $1 text x
-- @test
UPDATE up_t SET n = 0 WHERE name = $1 OR $1 IS NULL RETURNING id;
