-- abs preserves the argument's type (int/float); one-arg ceil/round over int resolve
-- to the double precision overload (float8, not numeric); round(float) rounds to the
-- nearest integer in float8 (a tie to the even one: functions/round_float8_ties_to_even.sql).
-- @setup
CREATE TABLE t (id INT);
INSERT INTO t VALUES (1);
-- @test
SELECT abs(-7) AS ai, abs(-3.5::float8) AS af, ceil(5) AS ci,
       round(2.567::float8) AS rf, trunc(9.99::float8) AS tf
FROM t;
