-- The precision window's other end, `NUMERIC_MAX_PRECISION`. The inclusive end is
-- accepted — `types/refused_typmod_window_ends.sql`.
-- @expect-error: a NUMERIC precision above 1000
-- @expect-message
-- @test
SELECT 1.5::numeric(1001) AS never_reached;
