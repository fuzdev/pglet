-- The months pass lands in 4750 BC, before 18.6's timestamp range (4714-11-24 BC) but
-- after year -9999, and is refused there; the days pass would have come back to 0001.
-- @expect-error: timestamp out of range
-- @expect-message
-- @test
SELECT timestamp '0001-01-01' + interval '-57000 months 1735000 days';
