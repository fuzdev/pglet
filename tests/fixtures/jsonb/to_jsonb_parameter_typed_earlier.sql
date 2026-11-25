-- A cast analysed earlier types the `$N` before `to_jsonb` reads it.
-- @param $1 int4 4
-- @test
SELECT $1::int AS i, to_jsonb($1) AS j;
