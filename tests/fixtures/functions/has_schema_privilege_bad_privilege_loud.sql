-- `SELECT` is not a schema privilege. `22023` on both.
-- @test
-- @expect-error: unrecognized privilege type: "SELECT"
SELECT has_schema_privilege('public', 'SELECT');
