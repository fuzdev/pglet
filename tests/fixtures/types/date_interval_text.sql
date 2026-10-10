-- Text-format guard for `date + interval → timestamp`: the binary default
-- wouldn't catch a text-only rendering bug, and the result is a `timestamp`
-- without a time zone, so it must render with no `+00` offset (unlike a
-- `timestamptz`).
-- @text
-- @test
SELECT '2024-06-15'::date + INTERVAL '3 days' AS ts;
