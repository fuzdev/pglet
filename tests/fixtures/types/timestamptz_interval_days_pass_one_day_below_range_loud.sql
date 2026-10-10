-- `timestamptz`'s days pass lets Julian day -1 through its own check and then refuses
-- it as a timestamp at UTC, so the floor is the same as `timestamp`'s.
-- @expect-error: timestamp out of range
-- @expect-message
-- @test
SELECT timestamptz '0001-01-01 00:00:00+00' + interval '-1721427 days 41314248 hours';
