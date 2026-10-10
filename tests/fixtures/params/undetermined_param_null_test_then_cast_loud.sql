-- A cast later in the statement does not rescue an earlier null test: the reference
-- under `IS NULL` was already built untyped.
-- @setup
CREATE TABLE up_t (id INT, name TEXT, n BIGINT);
INSERT INTO up_t VALUES (1, 'x', 10), (2, 'y', 20), (3, NULL, 30);
-- @param $1 text NULL
-- @test
-- @expect-error: the null test is reached before the occurrence that types the parameter: 42P08
-- @expect-message
SELECT $1 IS NULL AS b, $1::text AS v;
