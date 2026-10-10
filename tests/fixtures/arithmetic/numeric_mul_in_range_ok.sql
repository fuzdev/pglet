-- A large but in-range numeric product (weight well under the format cap) still
-- computes exactly — the weight cap must not over-reject. `10^1000 * 10^1000`
-- equals `10^2000`, a 2001-digit result.
-- @test
SELECT
  (10::numeric ^ 1000) * (10::numeric ^ 1000) = 10::numeric ^ 2000 AS in_range_ok,
  length(((10::numeric ^ 1000) * (10::numeric ^ 1000))::text) AS product_len;
