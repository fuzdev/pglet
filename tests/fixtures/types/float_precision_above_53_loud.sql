-- `FLOAT(54)` is above 18.6's window. The other end of
-- `types/float_precision_zero_loud.sql`'s refusal, and a different sentence.
-- @expect-error: a float precision of 54 bits or more
-- @expect-message
-- @test
SELECT 1::float(54) AS never_reached;
