-- The negative overflow is refused too, not answered `-Infinity`.
-- @test
-- @expect-error: "-1e39" is out of range for type real (22003)
-- @expect-message
SELECT '-1e39'::real;
