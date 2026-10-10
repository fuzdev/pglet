-- A typed constant reads its offset through the same reader as a cast.
-- @expect-error: time zone displacement out of range
-- @expect-message
-- @test
SELECT timestamptz '2000-01-01 00:00:00+16:00';
