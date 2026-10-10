-- A literal that ends before its closing brace is malformed, after its elements convert.
-- @test
-- @expect-error: malformed array literal, 22P02
SELECT '{1,2'::int4[];
