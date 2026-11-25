-- Years folded into months past `integer` is `22008`.
-- @expect-error: interval out of range
-- @test
SELECT make_interval(years => 2147483647, months => 1);
