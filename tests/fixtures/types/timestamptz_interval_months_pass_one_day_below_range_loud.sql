-- `timestamptz`'s months pass has the same floor: Julian day -1, one day before 18.6's
-- timestamp range, is refused though the days pass would come back.
-- @expect-error: timestamp out of range
-- @expect-message
-- @test
SELECT timestamptz '0001-01-23 00:00:00+00' + interval '-56558 months 1721449 days';
