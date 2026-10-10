-- A declared `date` `$N` beside an `integer` literal has no common type: `42804`, not a
-- refused conversion's `42846`.
-- @param-declared $1 date 2000-01-01
-- @expect-error: COALESCE types date and integer cannot be matched
-- @test
SELECT coalesce($1, 1);
