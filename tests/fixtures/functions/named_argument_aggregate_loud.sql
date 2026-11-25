-- An aggregate called with a named argument resolves none either.
-- @expect-error: no count(a => integer)
-- @setup
CREATE TABLE na_agg (a INT);
-- @test
SELECT count(a => 1) FROM na_agg;
