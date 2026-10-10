-- A trailing comma leaves an empty item. `22023` on both.
-- @test
-- @expect-error: unrecognized privilege type: ""
SELECT has_schema_privilege('public', 'USAGE,');
