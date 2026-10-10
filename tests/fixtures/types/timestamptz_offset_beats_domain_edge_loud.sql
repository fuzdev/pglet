-- The offset's verdict comes before the instant's range: this literal's UTC reading
-- would be before year 1.
-- @expect-error: time zone displacement out of range
-- @expect-message
-- @test
SELECT '0001-01-01T00:00:00+23:59'::timestamptz;
