-- A comparison 18.6 does not fold — `date < timestamptz` is a stable cross-type operator,
-- evaluated at run time — decides nothing while the aggregates are collected, so the
-- `sum` is kept and runs. Treating it as a constant would drop the aggregate and answer
-- `t` where 18.6 raises.
-- @setup
CREATE TABLE u (a INT, b INT NOT NULL, y TEXT);
INSERT INTO u VALUES (1, 10, 'x'), (2, 0, 'z');
-- @expect-error: division by zero
-- @test
SELECT '2020-01-01'::date < '2021-01-01 00:00:00+00'::timestamptz OR sum(1/b) > 0 FROM u;
