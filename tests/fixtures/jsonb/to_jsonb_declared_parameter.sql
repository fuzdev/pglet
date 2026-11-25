-- A `$N` whose type the client declares is converted by that type.
-- @param-declared $1 int4 7
-- @test
SELECT to_jsonb($1) AS j;
