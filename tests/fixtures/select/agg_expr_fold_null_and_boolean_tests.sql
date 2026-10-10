-- `IS [NOT] NULL` and `IS [NOT] TRUE | FALSE | UNKNOWN` over a certain decider are
-- constants to 18.6's planner too, a `NULL` operand included. Every column answers `0`:
-- a test that holds drops the `ELSE` aggregate, one that does not drops the `THEN` one.
-- @setup
CREATE TABLE u (a INT, b INT NOT NULL, y TEXT);
INSERT INTO u VALUES (1, 10, 'x'), (2, 0, 'z');
-- @test
SELECT CASE WHEN NULL IS NULL THEN 0 ELSE sum(1/b) END AS null_is_null,
  CASE WHEN (1 = 1) IS NULL THEN sum(1/b) ELSE 0 END AS true_is_null,
  CASE WHEN true IS NOT NULL THEN 0 ELSE sum(1/b) END AS true_is_not_null,
  CASE WHEN NULL IS NOT NULL THEN sum(1/b) ELSE 0 END AS null_is_not_null,
  CASE WHEN (1 < 2) IS TRUE THEN 0 ELSE sum(1/b) END AS is_true,
  CASE WHEN NULL IS TRUE THEN sum(1/b) ELSE 0 END AS null_is_true,
  CASE WHEN false IS NOT TRUE THEN 0 ELSE sum(1/b) END AS is_not_true,
  CASE WHEN true IS NOT TRUE THEN sum(1/b) ELSE 0 END AS true_is_not_true,
  CASE WHEN (1 > 2) IS FALSE THEN 0 ELSE sum(1/b) END AS is_false,
  CASE WHEN NULL IS FALSE THEN sum(1/b) ELSE 0 END AS null_is_false,
  CASE WHEN NULL IS NOT FALSE THEN 0 ELSE sum(1/b) END AS null_is_not_false,
  CASE WHEN false IS NOT FALSE THEN sum(1/b) ELSE 0 END AS false_is_not_false,
  CASE WHEN NULL IS UNKNOWN THEN 0 ELSE sum(1/b) END AS is_unknown,
  CASE WHEN true IS UNKNOWN THEN sum(1/b) ELSE 0 END AS true_is_unknown,
  CASE WHEN false IS NOT UNKNOWN THEN 0 ELSE sum(1/b) END AS is_not_unknown,
  CASE WHEN NULL IS NOT UNKNOWN THEN sum(1/b) ELSE 0 END AS null_is_not_unknown,
  CASE WHEN NOT NULL THEN sum(1/b) ELSE 0 END AS not_null_literal,
  CASE WHEN NOT (1 > 2) THEN 0 ELSE sum(1/b) END AS not_comparison
FROM u;
