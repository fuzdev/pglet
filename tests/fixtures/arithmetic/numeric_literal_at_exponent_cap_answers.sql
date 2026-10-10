-- The control beside the exponent-cap refusal, and the cell that separates the cap
-- from a range check on the value: `PG_INT32_MAX / 2` is the largest exponent 18.6's
-- reader accumulates, so at it a literal whose value is **zero** answers `0` — the
-- weight it computes is never asked about. One more is
-- `arithmetic/numeric_literal_over_exponent_cap_loud.sql`, refused whatever the value.
-- @test
SELECT 0e1073741823 AS zero_at_the_exponent_cap;
