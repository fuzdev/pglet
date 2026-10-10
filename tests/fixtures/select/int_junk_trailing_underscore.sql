-- A trailing `_` digit-group separator (no digit after it) is trailing junk:
-- `1000_` is a lex error, not `1000` followed by a column `_`. A `_` separator is
-- only valid *between* two digits. SQLSTATE 42601.
-- @test
-- @expect-error: trailing junk after numeric literal (trailing underscore)
SELECT 1000_;
