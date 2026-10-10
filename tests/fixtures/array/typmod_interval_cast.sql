-- `interval` carries a modifier, so `interval(3)[]` is an array — the spelling used
-- to lose both the `[]` and the modifier and try to parse the whole literal as one
-- scalar interval. The literal has no sub-second field, so the precision has nothing
-- to round here; `array/precision_array_cast.sql` is where the rounding itself is
-- asserted.
-- @test
SELECT '{1 day,2 hours}'::interval(3)[] AS iv;
