-- `numeric::float8` reads the numeric's text with `float8in`: past the range it is refused,
-- never `Infinity`.
-- @test
-- @expect-error: out of range for type double precision (22003)
-- @expect-message
SELECT 1e400::numeric::float8;
