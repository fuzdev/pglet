-- A fold read beside a column whose type is right and not certain takes a computed
-- `double precision` sibling as its target, so the set is `double precision` as on
-- 18.6 and the sibling keeps its value. It was cast to the column's declared type:
-- `integer` over a FROM `unnest` of an `ARRAY[…]` constructor, rounding `1.5` to `2`,
-- and `numeric` over a column defined by an exact `avg` beside a guessed column's
-- aggregate, where the sibling kept fifteen digits. The `double precision` branch
-- decides the target with no branch typed beside it too — the sibling computed over the
-- guessed column — where the set was left unresolved, declared `integer` over the
-- `double precision` value.
-- @setup
CREATE TABLE t (k INT, i INT, f FLOAT8, d DATE, d2 DATE);
INSERT INTO t VALUES (1, 1, 0.1234567890123456789, '2020-01-03', '2020-01-01'),
  (2, NULL, 1.5, NULL, NULL);
-- @test
SELECT k, COALESCE(u.x, t.f + 0) AS c, greatest(u.x, t.f + 0) AS g,
  CASE WHEN u.x IS NULL THEN t.f * 1 ELSE u.x END AS w, nullif(t.f + 0, u.x) AS n,
  nullif(u.x, t.f + 0) AS n2, COALESCE(u.x, t.f + 0, NULL) AS with_null,
  COALESCE(u.x * 1, t.f + 0) AS computed_sibling, greatest(abs(u.x), t.f + 0) AS computed_g,
  CASE WHEN u.x IS NULL THEN t.f * 1 ELSE -u.x END AS computed_w,
  COALESCE(u.x, t.f + 1.375, u.x * 1) AS three, COALESCE(m.x, t.f + 0) AS moved,
  greatest(m.x, t.f + 0) AS moved_g, nullif(t.f + 0, m.x) AS moved_n
FROM t, unnest(ARRAY[NULL::int]) u(x),
  (SELECT avg(v) + max(g.y) AS x FROM (VALUES (1), (4)) q(v), unnest(ARRAY[0]) g(y)
   WHERE false) m
ORDER BY k;
