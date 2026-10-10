-- The schema is resolved before the privilege string is parsed. `3F000` on both.
-- @test
-- @expect-error: schema "nosuch" does not exist
SELECT has_schema_privilege('nosuch', 'SELECT');
