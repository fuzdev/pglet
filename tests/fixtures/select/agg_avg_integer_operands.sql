-- An integer `avg` is a `numeric` operand to every operator and cast over it: `+ 1`,
-- `* 2.5`, `/ 3`, `% 2`, `^ 2` and unary `-` keep `numeric` (`%` has no `float8` form,
-- so a `float8` `avg` is refused there), `::int` rounds it, `||` renders its digits, and
-- a `HAVING` compares it as a `numeric`; `sum(bigint)` is `numeric` under `+ 1` and `/ 2`.
-- @setup
CREATE TABLE t (i INT, b BIGINT);
INSERT INTO t VALUES (1, 9223372036854775807), (2, 9223372036854775806);
-- @test
SELECT avg(i) + 1 AS p, avg(i) * 2.5 AS m, avg(i) / 3 AS d, avg(i) % 2 AS r, avg(i) ^ 2 AS e,
  -avg(i) AS n, avg(i)::int AS c, avg(i) || 'x' AS cat, sum(b) + 1 AS sb, sum(b) / 2 AS hb
FROM t HAVING avg(i) > 1.4;
