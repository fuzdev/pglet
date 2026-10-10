-- `numeric::real` reads the numeric's text with `float4in`, so the message names its digits.
-- @test
-- @expect-error: "1000000000000000000000000000000000000000" is out of range for type real (22003)
-- @expect-message
SELECT 1e39::numeric::real;
