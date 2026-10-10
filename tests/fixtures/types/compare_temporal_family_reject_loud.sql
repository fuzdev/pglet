-- `date = time` has no operator (`42883`): `time` is not a member of the
-- date/timestamp family, so the gate must not widen the family to every temporal
-- type. (`timestamp = interval` and `time = timestamp` are likewise rejected.)
-- @setup
CREATE TABLE t (d DATE, tm TIME);
INSERT INTO t VALUES ('2024-06-15', '00:00:00');
-- @test
-- @expect-error: no date = time operator
SELECT * FROM t WHERE d = tm;
