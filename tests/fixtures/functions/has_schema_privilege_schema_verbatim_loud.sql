-- The schema name is taken verbatim, not folded. `3F000` on both.
-- @test
-- @expect-error: schema "PUBLIC" does not exist
SELECT has_schema_privilege('PUBLIC', 'USAGE');
