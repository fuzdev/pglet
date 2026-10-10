-- A declared `boolean` `$N` beside an `integer` literal has no common type:
-- `42804 COALESCE types boolean and integer cannot be matched`, at `Parse`.
-- @param-declared $1 bool true
-- @expect-error: COALESCE types boolean and integer cannot be matched
-- @test
SELECT coalesce($1, 1);
