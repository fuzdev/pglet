-- An `INSERT` target column types the parameter; `RETURNING` is analysed after the
-- row source.
-- @setup
CREATE TABLE up_t (id INT, name TEXT, n BIGINT);
INSERT INTO up_t VALUES (1, 'x', 10), (2, 'y', 20), (3, NULL, 30);
-- @param $1 text z
-- @test
INSERT INTO up_t (id, name) VALUES (9, $1) RETURNING $1 IS NULL AS b;
