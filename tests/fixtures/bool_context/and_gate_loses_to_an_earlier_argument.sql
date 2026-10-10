-- The mirror spelling: the left operand's operand-type failure is positionally
-- first, so it is the one reported.
-- @expect-error: the LIKE operand's 42883, not the AND gate's 42804
-- @test
CREATE TABLE bcint3 (a INT, b BIGINT, CHECK (b LIKE 'x' AND a));
