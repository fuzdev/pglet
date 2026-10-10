-- The other route to 23514: `ALTER TABLE … ADD CONSTRAINT` validating existing rows.
-- Different message, and **no** DETAIL — no single row is "the new row".
-- @setup
CREATE TABLE t_c3 (a INT);
INSERT INTO t_c3 VALUES (-1);

-- @test
-- @expect-error: an existing row violates the new CHECK
-- @expect-message
ALTER TABLE t_c3 ADD CONSTRAINT t_c3_pos CHECK (a > 0);
