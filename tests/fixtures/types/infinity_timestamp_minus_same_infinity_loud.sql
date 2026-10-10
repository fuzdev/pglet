-- An infinity less the same infinity has no interval on either engine: `timestamp_mi`'s
-- `interval out of range`.
-- @expect-error: interval out of range
-- @expect-message
-- @test
SELECT timestamptz 'infinity' - timestamptz 'infinity';
