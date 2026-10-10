-- The `*` operator (`numeric_mul`) caps the product's display scale at
-- NUMERIC_DSCALE_MAX (16383): squaring `0.1` doubles the scale each step, and once
-- the exact product would need more than 16383 fractional digits it rounds — a
-- value below that precision folds to exactly 0, matching Postgres. Level 13
-- (dscale 8192) is still nonzero; level 14 (exact dscale would be 16384) is 0.
-- @test
WITH RECURSIVE sq(n, val) AS (
  SELECT 0, 0.1::numeric
  UNION ALL
  SELECT n + 1, val * val FROM sq WHERE n < 14
)
SELECT n, (val = 0) AS is_zero FROM sq WHERE n IN (13, 14) ORDER BY n;
