-- A bare select-list `$1` becomes `text` only after the whole query level is analysed,
-- so it does not rescue a null test anywhere in that level.
-- @setup
CREATE TABLE up_t (id INT, name TEXT, n BIGINT);
INSERT INTO up_t VALUES (1, 'x', 10), (2, 'y', 20), (3, NULL, 30);
-- @param $1 text NULL
-- @test
-- @expect-error: the null test is reached before the occurrence that types the parameter: 42P08
-- @expect-message
SELECT $1 AS p, $1 IS NULL AS b FROM up_t;
