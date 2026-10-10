-- A nested literal holding no element at all is the empty array, as on Postgres: there is
-- no second dimension to refuse.
-- @test
SELECT '{{}}'::text[] AS a, '{{},{}}'::int4[] AS b, '{ { } }'::bytea[] AS c, '{}'::text[] AS d;
