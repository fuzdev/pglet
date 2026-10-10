-- A non-`public` schema qualifier fails loud (42P01): pglet models one implicit
-- schema, so `noschema.x` is a relation that doesn't exist — matching Postgres,
-- instead of silently dropping the qualifier and resolving `x`.
-- @expect-error: relation "noschema.x" does not exist (42P01)
-- @test
SELECT * FROM noschema.x;
