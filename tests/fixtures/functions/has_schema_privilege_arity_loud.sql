-- No one-argument overload. `42883` on both.
-- @test
-- @expect-error: function has_schema_privilege(unknown) does not exist
SELECT has_schema_privilege('public');
