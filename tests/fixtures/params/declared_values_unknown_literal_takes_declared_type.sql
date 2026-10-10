-- A `VALUES` column over a declared `$N` and a string constant is the declared type.
-- @param-declared $1 int4 5
-- @test
SELECT * FROM (VALUES ($1), ('6')) v(a);
