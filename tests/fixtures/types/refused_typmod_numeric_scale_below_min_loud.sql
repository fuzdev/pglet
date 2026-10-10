-- The scale window is symmetric where the precision's is not (`NUMERIC_MIN_SCALE` is
-- `-1000`), and this end is the one that was **silent**: `1.5::numeric(5,-1001)` answered
-- `0` here, a rounding nobody is allowed to spell.
-- @expect-error: a NUMERIC scale below -1000
-- @expect-message
-- @test
SELECT 1.5::numeric(5,-1001) AS never_reached;
