-- A `date` less a `date` is an `integer`, and a branch over one is typed: a fold holding
-- it beside a `double precision` branch is 18.6's `double precision`, and beside a
-- `numeric` one `numeric`. Untyped, the branch left its set unresolved — declared
-- `integer` over the other branch's value — and under an exact `avg` it declared
-- `numeric` over `double precision`.
-- @setup
CREATE TABLE t (k INT, i INT, f FLOAT8, n NUMERIC, d DATE, d2 DATE);
INSERT INTO t VALUES (1, 1, 0.1234567890123456789, 1.5, '2020-01-05', '2020-01-01'),
  (2, NULL, 1e300, NULL, NULL, NULL), (3, 3, 2.5, 2.25, '2020-01-03', '2020-01-01');
-- @test
SELECT k, COALESCE(d - d2, f + 0) AS c, COALESCE((d - d2) * 2, n * 1) AS exact,
  CASE WHEN k = 1 THEN d - d2 ELSE f + 0 END AS w, greatest(d - d2, f + 0) AS g,
  nullif(f, d - d2) AS n1, ARRAY[d - d2, f * 1] AS a, COALESCE(abs(d - d2), i) AS same,
  (SELECT COALESCE(avg(v.x) + (max(u.d) - max(u.d2)), max(u.f) + 0)
     FROM (SELECT count(*) AS x FROM t GROUP BY k) v, t u WHERE u.k = t.k) AS under_avg
FROM t ORDER BY k;
