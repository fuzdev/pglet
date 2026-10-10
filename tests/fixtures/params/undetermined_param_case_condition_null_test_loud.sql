-- A `CASE` condition holding the null test, the comparison in a later branch.
-- @setup
CREATE TABLE up_t (id INT, name TEXT, n BIGINT);
INSERT INTO up_t VALUES (1, 'x', 10), (2, 'y', 20), (3, NULL, 30);
-- @param $1 text NULL
-- @test
-- @expect-error: the null test is reached before the occurrence that types the parameter: 42P08
-- @expect-message
SELECT count(*) AS c FROM up_t WHERE CASE WHEN $1 IS NULL THEN true ELSE name = $1 END;
