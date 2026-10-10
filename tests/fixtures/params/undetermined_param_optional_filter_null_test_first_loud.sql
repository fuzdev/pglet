-- The optional-filter idiom with the null test written first. 18.6 gives a parameter
-- reference the type the parameter has when the reference is analysed: here the null test
-- is reached while `$1` is still untyped, the comparison then types it, and the statement
-- is refused `42P08` because one reference disagrees with the parameter's type.
-- @setup
CREATE TABLE up_t (id INT, name TEXT, n BIGINT);
INSERT INTO up_t VALUES (1, 'x', 10), (2, 'y', 20), (3, NULL, 30);
-- @param $1 text NULL
-- @test
-- @expect-error: the null test is reached before the occurrence that types the parameter: 42P08
-- @expect-message
SELECT count(*) AS c FROM up_t WHERE ($1 IS NULL OR name = $1);
