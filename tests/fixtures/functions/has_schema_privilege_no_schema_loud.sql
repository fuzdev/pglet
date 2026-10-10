-- A schema that does not exist. `3F000` on both.
-- @test
-- @expect-error: schema "nosuch" does not exist
SELECT has_schema_privilege('nosuch', 'USAGE');
