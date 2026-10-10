-- The key position of `jsonb_build_object` is `"any"` too.
-- @setup
CREATE TABLE up_t (id INT, name TEXT, n BIGINT);
INSERT INTO up_t VALUES (1, 'x', 10), (2, 'y', 20), (3, NULL, 30);
-- @param $1 text k
-- @test
-- @expect-error: 18.6 leaves the parameter untyped: 42P18
-- @expect-message
SELECT jsonb_build_object($1, 1) AS j;
