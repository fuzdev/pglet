-- A `real` read from text is range-checked: a decimal past the type's range is refused,
-- never the `Infinity` it rounds to.
-- @test
-- @expect-error: "1e39" is out of range for type real (22003)
-- @expect-message
SELECT '1e39'::real;
