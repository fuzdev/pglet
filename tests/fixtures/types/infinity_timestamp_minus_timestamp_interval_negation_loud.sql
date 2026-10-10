-- The interval negation is judged before the infinity is kept, as `timestamp_mi_interval`
-- orders it.
-- @expect-error: interval out of range
-- @expect-message
-- @test
SELECT timestamp 'infinity' - interval '-2147483648 days';
