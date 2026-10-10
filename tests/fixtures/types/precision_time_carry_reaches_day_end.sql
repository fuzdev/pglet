-- A `time` rounding carry is **not** wrapped: `TimeADT` is a raw microsecond count and
-- 18.6 lets it reach `24:00:00`, which pglet's `Value::Time` already represents and
-- prints.
--
-- The two columns carry from **different** places, which is why both are here. `a`'s
-- carry is the typmod's, at `p = 3`. `b`'s is the *input* function's own rounding to
-- microseconds, at `p = 6` where the typmod is a no-op — the control that says the
-- endpoint is reachable without this rule and was before it.
-- @test
SELECT '23:59:59.9999'::time(3) AS a, '23:59:59.9999995'::time(6) AS b;
