-- The months pass lands on 4714-11-23 BC, Julian day -1, one day before 18.6's
-- timestamp range; from 0001-01-24 the same span is a value.
-- @expect-error: timestamp out of range
-- @expect-message
-- @test
SELECT timestamp '0001-01-23' + interval '-56558 months 1721449 days';
