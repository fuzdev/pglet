-- `greatest`, `least` and `nullif` over a declared `float8` `$N` beside an `integer`
-- literal resolve over `float8` and keep `1.5`.
-- @param-declared $1 float8 1.5
-- @test
SELECT greatest($1, 1) AS g, least($1, 10) AS l, nullif($1, 0) AS n;
