-- A well-formed nested literal fails loud for every element type, and says what is
-- true: the literal is not malformed, the second dimension is unsupported.
-- @test
-- The `::TEXT` keeps Postgres's side decodable by the harness.
-- @expect-error: pglet does not support multidimensional arrays (Postgres accepts this)
SELECT '{{1,2},{3,4}}'::int4[]::TEXT;
