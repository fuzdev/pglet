-- Two comparison-first optional filters over two column types in one statement.
-- @setup
CREATE TABLE up_t (id INT, name TEXT, n BIGINT);
INSERT INTO up_t VALUES (1, 'x', 10), (2, 'y', 20), (3, NULL, 30);
-- @param $1 text NULL
-- @param $2 int8 20
-- @test
SELECT id FROM up_t WHERE (name = $1 OR $1 IS NULL) AND (n = $2 OR $2 IS NULL) ORDER BY id;
