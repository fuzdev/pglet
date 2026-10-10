-- A parameter declared `text` is a `text` argument: no signature.
-- @param-declared $1 text 3
-- @expect-error: function generate_series(integer, text) does not exist
-- @test
SELECT * FROM generate_series(1, $1);
