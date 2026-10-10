-- The whitespace around a float's text is C's `isspace` set, so a no-break space (U+00A0, written as an escape) is text,
-- not padding, and the input is malformed.
-- @test
-- @expect-error: invalid input syntax for type double precision (22P02)
SELECT E'\u00a01'::float8;
