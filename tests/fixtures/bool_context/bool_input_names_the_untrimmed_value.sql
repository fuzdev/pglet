-- `boolin` trims for the parse and reports `in_str`, the original — so the
-- message carries the whitespace, and this fixture locks the message byte for
-- byte because it is the message that is the subject.
-- @expect-error: invalid input syntax for type boolean: "  zz  "
-- @expect-message
-- @test
SELECT '  zz  '::bool;
