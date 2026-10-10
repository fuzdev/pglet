-- Digit-group underscores (Postgres 16+): a `_` between digits is a separator in
-- the integer, fraction, or exponent part. `1_000` is an integer; `1_000.5` and
-- `1_0.5e1_0` are numeric — the underscores are stripped before typing/parsing.
-- @test
SELECT
  1_000 AS a,
  1_000_000 AS b,
  1_000.5 AS c,
  1_0.5e1_0 AS d;
