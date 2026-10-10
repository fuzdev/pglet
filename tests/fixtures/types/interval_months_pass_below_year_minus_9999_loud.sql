-- The months pass lands in 10001 BC, below 18.6's timestamp range.
-- @expect-error: timestamp out of range
-- @expect-message
-- @test
SELECT timestamp '0001-01-01' + interval '-120012 months 4383000 days';
