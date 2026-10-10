-- Large-operand numeric `%`: the truncated quotient must be computed by EXACT
-- integer division, not `div().trunc(0)` (whose select_div_scale rounding gives
-- the wrong remainder above ~10^16). Values verified against Postgres.
-- @test
SELECT
  100000000000000001::numeric % 3 AS a,
  100000000000000007::numeric % 7 AS b,
  (-100000000000000001)::numeric % 3 AS c;
