-- `date - interval` answers a timestamp, and past 18.6's range its refusal is the
-- timestamp sentence, not the date one.
-- @expect-error: timestamp out of range
-- @expect-message
-- @test
SELECT date '0001-01-01' - interval '10000 years';
