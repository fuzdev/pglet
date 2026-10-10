-- `avg` over a `date` less a `date` is the exact `numeric` of an `integer` argument, and
-- `sum` `bigint` — directly, under arithmetic, through a derived table, and as the
-- branch of a fold. The difference was an argument of no certain type, and its `avg`
-- `double precision`.
-- @setup
CREATE TABLE t (k INT, i INT, d DATE, d2 DATE);
INSERT INTO t VALUES (1, 1, '2020-01-05', '2020-01-01'), (2, NULL, NULL, NULL),
  (3, 3, '2020-01-03', '2020-01-01'), (4, 4, '2020-01-02', '2020-01-01');
-- @test
SELECT a.*, q1.derived, q2.grouped
FROM (SELECT avg(d - d2) AS a, sum(d - d2) AS s, round(avg(d - d2), 2) AS r,
        avg((d - d2) * 2) AS doubled, avg(COALESCE(d - d2, i)) AS folded FROM t) a,
  (SELECT avg(x) AS derived FROM (SELECT d - d2 AS x FROM t) q) q1,
  (SELECT avg(x) AS grouped FROM (SELECT max(d) - min(d2) AS x FROM t GROUP BY k) q) q2;
