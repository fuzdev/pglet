-- `timestamp - interval` negates the interval before adding it, and a field that
-- cannot be negated is `interval out of range`, ahead of any pass.
-- @expect-error: interval out of range
-- @expect-message
-- @test
SELECT timestamp '2000-01-01' - interval '-2147483648 days';
