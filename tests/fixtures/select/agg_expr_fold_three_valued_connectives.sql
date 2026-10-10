-- A connective of two certain operands is a certain decider by SQL's three-valued
-- table: `NULL AND true` and `NULL OR false` are `NULL`, not their right operand and not
-- `false`. Every column answers `0`: each condition is `NULL` or `false`, so the arm's
-- `sum` over the `b = 0` row is dropped — and under `NOT`, a `NULL` taken for `false`
-- would keep it.
-- @setup
CREATE TABLE u (a INT, b INT NOT NULL, y TEXT);
INSERT INTO u VALUES (1, 10, 'x'), (2, 0, 'z');
-- @test
SELECT CASE WHEN (NULL AND true) THEN sum(1/b) ELSE 0 END AS null_and_true,
  CASE WHEN NOT (NULL AND true) THEN sum(1/b) ELSE 0 END AS not_null_and_true,
  CASE WHEN (NULL OR false) THEN sum(1/b) ELSE 0 END AS null_or_false,
  CASE WHEN NOT (NULL OR false) THEN sum(1/b) ELSE 0 END AS not_null_or_false,
  CASE WHEN (true AND NULL) THEN sum(1/b) ELSE 0 END AS true_and_null,
  CASE WHEN NOT (false OR NULL) THEN sum(1/b) ELSE 0 END AS not_false_or_null,
  CASE WHEN (NULL AND NULL) THEN sum(1/b) ELSE 0 END AS null_and_null,
  CASE WHEN (true AND false) THEN sum(1/b) ELSE 0 END AS true_and_false,
  CASE WHEN (false OR false) THEN sum(1/b) ELSE 0 END AS false_or_false
FROM u;
