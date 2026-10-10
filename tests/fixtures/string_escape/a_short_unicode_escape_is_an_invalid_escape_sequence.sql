-- Too few hex digits after `\u` inside an `E'...'` is 18.6's `{xeunicodefail}` rule,
-- which raises `ERRCODE_INVALID_ESCAPE_SEQUENCE` -- `22025`, not a syntax error. The
-- same malformation inside a `U&'...'` is `42601`; the two are deliberately not
-- unified.
-- @test
-- @expect-error: a short Unicode escape in an E-string is 22025
-- @expect-message
SELECT E'\u00';
