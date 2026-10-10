-- Exponentiation `^`: `int ^ int → float8` (no integer operator exists), left-
-- associative, binding tighter than `* / %` and looser than unary minus (so
-- `-2 ^ 2` is `(-2) ^ 2 = 4`, matching Postgres). `numeric ^ <integer>` stays
-- numeric with Postgres's power scale.
-- @test
SELECT
  2 ^ 3 AS a,
  2 ^ 3 ^ 2 AS b,
  2 * 3 ^ 2 AS c,
  -2 ^ 2 AS d,
  2.0 ^ 3 AS e,
  2.5 ^ 2 AS f,
  2 ^ (-1) AS g;
