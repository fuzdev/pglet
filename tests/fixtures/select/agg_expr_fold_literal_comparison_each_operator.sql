-- A comparison of two integer or `numeric` literals is a constant to 18.6's planner
-- under each of the six operators, across the integer widths and `numeric`: where it
-- holds, the `ELSE` aggregate is dropped, and where it does not, the `THEN` one is.
-- Every column answers `0`; a comparison decided the wrong way, or not taken for a
-- decider, runs a `sum` over the `b = 0` row. The last two are compared exactly, where a
-- float would call them equal.
-- @setup
CREATE TABLE u (a INT, b INT NOT NULL, y TEXT);
INSERT INTO u VALUES (1, 10, 'x'), (2, 0, 'z');
-- @test
SELECT CASE WHEN 1 = 1 THEN 0 ELSE sum(1/b) END AS eq, CASE WHEN 1 = 2 THEN sum(1/b) ELSE 0 END AS eq_no,
  CASE WHEN 1 <> 2 THEN 0 ELSE sum(1/b) END AS ne, CASE WHEN 1 <> 1 THEN sum(1/b) ELSE 0 END AS ne_no,
  CASE WHEN 1 < 2 THEN 0 ELSE sum(1/b) END AS lt, CASE WHEN 2 < 2 THEN sum(1/b) ELSE 0 END AS lt_no,
  CASE WHEN 2 <= 2 THEN 0 ELSE sum(1/b) END AS le, CASE WHEN 3 <= 2 THEN sum(1/b) ELSE 0 END AS le_no,
  CASE WHEN 3 > 2 THEN 0 ELSE sum(1/b) END AS gt, CASE WHEN 2 > 2 THEN sum(1/b) ELSE 0 END AS gt_no,
  CASE WHEN 2 >= 2 THEN 0 ELSE sum(1/b) END AS ge, CASE WHEN 1 >= 2 THEN sum(1/b) ELSE 0 END AS ge_no,
  CASE WHEN 1 = 1.0 THEN 0 ELSE sum(1/b) END AS int_numeric,
  CASE WHEN 1.5 > 2 THEN sum(1/b) ELSE 0 END AS numeric_int,
  CASE WHEN 0.1 < 0.25 THEN 0 ELSE sum(1/b) END AS numerics,
  CASE WHEN 1 < 10000000000 THEN 0 ELSE sum(1/b) END AS int_bigint,
  CASE WHEN 10000000000 = 1e10 THEN 0 ELSE sum(1/b) END AS bigint_numeric,
  CASE WHEN -1 > 0 THEN sum(1/b) ELSE 0 END AS negative,
  CASE WHEN 16777217 = 16777216 THEN sum(1/b) ELSE 0 END AS past_float_precision,
  CASE WHEN 1.0000000001 > 1 THEN 0 ELSE sum(1/b) END AS numeric_exact
FROM u;
