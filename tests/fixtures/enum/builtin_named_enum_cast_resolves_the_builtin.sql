-- An enum may share a builtin's name — `CREATE TYPE int4` creates `public.int4` — and an
-- unqualified name still resolves to the builtin, `pg_catalog` being first on the search
-- path: every cast spelling converts to `integer`, not to the enum.
-- @setup
CREATE TYPE int4 AS ENUM ('k');

-- @test
SELECT '5'::int4 AS a, CAST('6' AS int4) + 1 AS b, int4 '7' AS c, 8::int4 AS d;
