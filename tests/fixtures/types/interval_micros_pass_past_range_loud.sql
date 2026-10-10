-- The days pass lands inside 18.6's timestamp range and the microseconds pass leaves it.
-- @expect-error: timestamp out of range
-- @expect-message
-- @test
SELECT timestamp '0001-01-01' + interval '100000000 days -9223372036854775807 microseconds';
