-- The days pass lands on Julian day -1, one day before 18.6's timestamp range, and is
-- refused there; `interval '-1721426 days 41314224 hours'` is a value.
-- @expect-error: timestamp out of range
-- @expect-message
-- @test
SELECT timestamp '0001-01-01' + interval '-1721427 days 41314248 hours';
