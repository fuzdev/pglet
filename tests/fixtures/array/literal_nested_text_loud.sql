-- **Multidimensional arrays are a pglet non-goal**: a well-formed nested literal, which
-- Postgres reads as a two-dimensional array, fails loud rather than being split at its
-- commas into elements that keep the inner braces.
-- @test
-- The `::TEXT` keeps Postgres's side decodable by the harness.
-- @expect-error: pglet does not support multidimensional arrays (Postgres accepts this)
SELECT '{{a,b}}'::text[]::TEXT;
