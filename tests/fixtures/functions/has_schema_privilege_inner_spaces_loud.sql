-- Only whitespace around an item is trimmed; two spaces inside one are not a spelling. `22023` on both.
-- @test
-- @expect-error: unrecognized privilege type: "USAGE WITH GRANT  OPTION"
SELECT has_schema_privilege('public', 'USAGE WITH GRANT  OPTION');
