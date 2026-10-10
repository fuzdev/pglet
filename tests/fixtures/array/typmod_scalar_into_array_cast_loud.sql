-- The array suffix is read even when the element's modifier is present, so a scalar
-- operand meets the array target and is refused (`42846`) rather than silently
-- casting to the element type.
-- @expect-error: an integer does not cast to numeric[]
-- @test
SELECT 1::numeric(4,1)[] AS never_reached;
