-- `10^131071` is the largest representable numeric (base-10000 weight =
-- NUMERIC_WEIGHT_MAX); one power higher overflows the format weight cap and fails
-- loud (Postgres 22003) instead of producing a 131073-digit result.
-- @test
-- @expect-error: numeric power weight overflow
SELECT 10::numeric ^ 131072;
