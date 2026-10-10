-- The SQL/JSON-path match operator `@@` is unimplemented in pglet — its `@@` token
-- isn't recognized by the lexer (only `@>` is), so it fails loud rather than
-- silently mis-lexing. Postgres accepts it (returns a boolean), so this is a
-- deliberately pglet-stricter case: any pglet error passes.
-- @test
-- @expect-error: @@ (jsonpath match) is not supported
SELECT '{"a":1}'::jsonb @@ '$.a == 1' AS r;
