-- Numeric math functions: abs/ceil/floor/round/trunc (exact over NUMERIC),
-- including the optional scale argument on round/trunc and negative values.
-- @setup
CREATE TABLE t (id INT, a NUMERIC);
INSERT INTO t VALUES (1, -1.50), (2, 2.567), (3, -2.5), (4, 1.1), (5, -1.5);
-- @test
SELECT id,
  abs(a) AS ab,
  ceil(a) AS ce,
  floor(a) AS fl,
  round(a) AS r0,
  round(a, 1) AS r1,
  trunc(a) AS t0,
  trunc(a, 1) AS t1
FROM t ORDER BY id;
