-- `numeric`'s half of the same position.
-- @expect-error: a NUMERIC precision below 1 at a column definition
-- @expect-message
-- @test
CREATE TABLE rtcn (c NUMERIC(0));
