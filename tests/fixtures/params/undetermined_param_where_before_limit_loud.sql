-- `LIMIT` types its parameter (`bigint`) after `WHERE` was analysed.
-- @setup
CREATE TABLE up_t (id INT, name TEXT, n BIGINT);
INSERT INTO up_t VALUES (1, 'x', 10), (2, 'y', 20), (3, NULL, 30);
-- @param $1 int8 1
-- @test
-- @expect-error: the null test is reached before the occurrence that types the parameter: 42P08
-- @expect-message
SELECT id FROM up_t WHERE $1 IS NULL LIMIT $1;
