-- A form feed is not among the whitespace the hex format skips, so it is refused as a digit.
-- @test
-- @expect-error: a form feed between hex pairs is 22023
SELECT E'\\x61\f62'::bytea;
