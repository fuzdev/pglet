-- A column a scalar subquery defines is of a certain type where the subquery's own
-- column is: `avg` over it is the exact `numeric` and `sum` of a `bigint` one `numeric`,
-- over values a `double precision` accumulation rounds. The subquery stands as the entry
-- itself, as an operand of arithmetic and of a unary minus, as a branch of a `COALESCE`,
-- a `CASE` and a `greatest`, as a `VALUES` cell and as a set operation's side.
-- @setup
CREATE TABLE t (k INT, b BIGINT);
INSERT INTO t VALUES (1, 1), (2, NULL), (3, 9007199254740993), (4, 2);
-- @test
SELECT
  (SELECT avg(x) FROM (SELECT (SELECT b FROM t t2 WHERE t2.k = t.k) AS x FROM t) q) AS entry_avg,
  (SELECT sum(x) FROM (SELECT (SELECT b FROM t t2 WHERE t2.k = t.k) AS x FROM t) q) AS entry_sum,
  (SELECT avg(x) FROM (SELECT (SELECT max(b) FROM t t2 WHERE t2.k <= t.k) AS x FROM t) q) AS agg_avg,
  (SELECT avg(x) FROM (SELECT (SELECT b FROM t t2 WHERE t2.k = t.k) + 0 AS x FROM t) q) AS operand,
  (SELECT sum(x) FROM (SELECT -(SELECT b FROM t t2 WHERE t2.k = t.k) AS x FROM t) q) AS negated,
  (SELECT avg(x) FROM (SELECT COALESCE((SELECT b FROM t t2 WHERE t2.k = t.k), 1) AS x FROM t) q)
    AS coalesce_branch,
  (SELECT avg(x) FROM
    (SELECT CASE WHEN k > 0 THEN (SELECT b FROM t t2 WHERE t2.k = t.k) ELSE 0 END AS x FROM t) q)
    AS case_branch,
  (SELECT sum(x) FROM (SELECT greatest((SELECT b FROM t t2 WHERE t2.k = t.k), 1) AS x FROM t) q)
    AS greatest_arg,
  (SELECT avg(x) FROM (VALUES ((SELECT max(b) FROM t)), (1), (1)) v(x)) AS values_cell,
  (SELECT avg(x) FROM (SELECT (SELECT max(b) FROM t) AS x UNION ALL SELECT 1 UNION ALL SELECT 1) q)
    AS setop_side;
