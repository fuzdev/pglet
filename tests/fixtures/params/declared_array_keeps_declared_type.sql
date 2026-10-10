-- `ARRAY[$1, 1]` over a declared `numeric` `$N` is a `numeric[]` holding `1.5`.
-- @param-declared $1 numeric 1.5
-- @test
SELECT ARRAY[$1, 1] AS a, ARRAY[$1] AS single;
