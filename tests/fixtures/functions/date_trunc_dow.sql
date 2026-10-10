-- `dow` is an EXTRACT-only (special-tier) field: date_trunc consults only the units
-- tier, so it is *unrecognized* (22023), not merely unsupported — matching Postgres,
-- which never calls DecodeSpecial for date_trunc.
-- @test
-- @expect-error: unit "dow" not recognized for date_trunc
SELECT date_trunc('dow', '2024-03-15 12:34:56'::timestamp);
