-- `concat_ws($1, $1)`: the separator types the parameter only when the call is
-- resolved, after the list argument's reference was built untyped.
-- @setup
CREATE TABLE up_t (id INT, name TEXT, n BIGINT);
INSERT INTO up_t VALUES (1, 'x', 10), (2, 'y', 20), (3, NULL, 30);
-- @param $1 text x
-- @test
-- @expect-error: the null test is reached before the occurrence that types the parameter: 42P08
-- @expect-message
SELECT concat_ws($1, $1) AS c;
