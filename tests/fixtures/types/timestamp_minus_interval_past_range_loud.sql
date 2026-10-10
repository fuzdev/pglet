-- `timestamp - interval` past 18.6's own range is its `timestamp out of range`, 22008.
-- The months pass lands in 10000 BC, below that range, and is refused there.
-- @expect-error: timestamp out of range
-- @expect-message
-- @test
SELECT timestamp '0001-01-01 00:00:00' - interval '10000 years';
