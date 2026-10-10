-- A function over `avg` of a `VALUES` list's integer column reads the exact `numeric`:
-- `round` rounds `2.5` away from zero, where a `double precision` average rounded it to
-- even and `round(…, 1)` had no `numeric` to take.
-- @test
SELECT round(avg(x)) AS r, round(avg(x), 1) AS r1, round(avg(x) * 3) AS r3, avg(x)::int AS c,
  trunc(avg(x)) AS t
FROM (VALUES (1), (4)) v(x);
