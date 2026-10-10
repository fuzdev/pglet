-- The months pass lands in 5000 BC, before 18.6's timestamp range (4714-11-24 BC), and
-- 18.6 refuses there even though the days pass would bring the result back to 0001.
-- @expect-error: timestamp out of range
-- @expect-message
-- @test
SELECT timestamp '0001-01-01' + interval '-60000 months 1826300 days';
