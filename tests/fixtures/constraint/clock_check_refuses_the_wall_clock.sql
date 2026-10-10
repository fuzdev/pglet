-- The mirror of `clock_check_reads_the_statements_instant`: `clock_timestamp()` is read at
-- the call, so a later read is not the transaction's start and the same `CHECK` refuses
-- it. The row is written after the transaction's first statement, so the wall clock has
-- moved past the transaction's start.
-- @setup
CREATE TABLE ccw (id INT, at TIMESTAMPTZ CHECK (at = now()));
-- @no-error-fields: the detail prints the failing row, which holds a wall-clock read no two runs share
-- @expect-error: the wall clock is past the transaction's start
-- @test
INSERT INTO ccw VALUES (1, clock_timestamp());
