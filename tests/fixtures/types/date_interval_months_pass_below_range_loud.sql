-- `date + interval` is `timestamp + interval` over the date's midnight: the months pass
-- lands in 5000 BC, before 18.6's timestamp range, and is refused there.
-- @expect-error: timestamp out of range
-- @expect-message
-- @test
SELECT date '0001-01-01' + interval '-60000 months 1826300 days';
