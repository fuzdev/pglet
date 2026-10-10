-- `count`'s argument is declared `"any"`, which takes an `unknown` parameter as it is.
-- 18.6 refuses the statement `42P18`.
-- @setup
CREATE TABLE up_t (id INT, name TEXT, n BIGINT);
INSERT INTO up_t VALUES (1, 'x', 10), (2, 'y', 20), (3, NULL, 30);
-- @param $1 text NULL
-- @test
-- @expect-error: 18.6 leaves the parameter untyped: 42P18
-- @expect-message
SELECT count($1) AS c FROM up_t;
