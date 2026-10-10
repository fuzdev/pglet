-- A `double precision` read from text is range-checked.
-- @test
-- @expect-error: "1e309" is out of range for type double precision (22003)
-- @expect-message
SELECT '1e309'::float8;
