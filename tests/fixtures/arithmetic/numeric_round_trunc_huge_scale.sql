-- `round`/`trunc` clamp a pathological scale to Postgres's numeric bounds
-- (numeric_round/numeric_trunc: up to NUMERIC_DSCALE_MAX = 16383) rather than
-- overflowing `weight + scale + 1` and silently zeroing the value. A huge positive
-- scale preserves the value at the max display scale (a 16385-char text: "1." plus
-- 16383 fractional digits); a huge negative scale rounds to 0.
-- @test
SELECT
  round(1.23::numeric, 2147483647) = 1.23 AS round_preserved,
  length(round(1.23::numeric, 2147483647)::text) AS round_len,
  trunc(1.23::numeric, 2147483647) = 1.23 AS trunc_preserved,
  length(trunc(1.23::numeric, 2147483647)::text) AS trunc_len,
  round(1.23::numeric, -2147483648) AS round_to_zero,
  trunc(1.23::numeric, -2147483648) AS trunc_to_zero;
