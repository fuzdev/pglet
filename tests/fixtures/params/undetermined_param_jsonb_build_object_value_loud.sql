-- `jsonb_build_object`'s arguments are a variadic `"any"` list. The cast spelling
-- `$1::text` is the one both engines answer.
-- @setup
CREATE TABLE up_t (id INT, name TEXT, n BIGINT);
INSERT INTO up_t VALUES (1, 'x', 10), (2, 'y', 20), (3, NULL, 30);
-- @param $1 text NULL
-- @test
-- @expect-error: 18.6 leaves the parameter untyped: 42P18
-- @expect-message
SELECT jsonb_build_object('k', $1) AS j;
