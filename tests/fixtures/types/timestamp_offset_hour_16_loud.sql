-- A plain `timestamp` drops a valid offset, but 18.6's `timestamp_in` still decodes it
-- first, so an out-of-range one is refused rather than ignored.
-- @expect-error: time zone displacement out of range
-- @expect-message
-- @test
SELECT '2000-01-01 00:00:00+16:00'::timestamp;
