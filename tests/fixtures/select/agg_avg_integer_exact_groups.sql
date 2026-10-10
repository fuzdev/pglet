-- `avg(integer)` is the exact `numeric` quotient at `select_div_scale`'s scale, per
-- `GROUP BY` group: `4.3333333333333333` (an `f64` prints `4.333333333333333`), the
-- trailing zeros of `1.5000000000000000` and `500001.000000000000`, a negative
-- quotient's rounding, and a NULL-only group's NULL. `sum(integer)` beside it is `int8`.
-- @setup
CREATE TABLE t (g INT, v INT);
INSERT INTO t VALUES (1, 3), (1, 4), (1, 6), (2, 1), (2, 2), (3, 1), (3, 1), (3, 1),
  (4, 1000000), (4, 2), (5, -7), (5, 1), (5, 1), (6, NULL), (7, 0), (7, NULL);
-- @test
SELECT g, avg(v) AS a, sum(v) AS s, count(v) AS n FROM t GROUP BY g ORDER BY g;
