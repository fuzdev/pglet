-- @test
-- A recursive `UNION` drops a row equal to one already accumulated, so the first
-- spelling is the one kept and the recursion stops there: a `numeric` scale either
-- way round, an interval's day/hour spelling, a float's signed zero.
WITH RECURSIVE
  numeric_a(x) AS (SELECT 1.0::numeric UNION SELECT x * 1.0 FROM numeric_a),
  numeric_b(x) AS (SELECT 1.00::numeric UNION SELECT round(x, 1) FROM numeric_b),
  day(x) AS (SELECT interval '1 day' UNION SELECT x - interval '1 day' + interval '24 hours' FROM day),
  zero(x) AS (SELECT '-0'::float8 UNION SELECT x * 0 FROM zero)
SELECT numeric_a.x AS numeric_a, numeric_b.x AS numeric_b, day.x AS day, zero.x AS zero
FROM numeric_a, numeric_b, day, zero;
