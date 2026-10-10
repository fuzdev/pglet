-- A value whose integer part exceeds NUMERIC(precision, scale)'s digit budget
-- fails loud (Postgres "numeric field overflow", 22003) rather than truncating.
-- @setup
CREATE TABLE t (amount NUMERIC(4, 2));
-- @expect-error: numeric field overflow (integer part too wide for NUMERIC(4,2))
-- @test
INSERT INTO t VALUES (123.45);
