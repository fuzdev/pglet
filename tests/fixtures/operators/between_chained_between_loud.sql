-- `BETWEEN` is `%nonassoc` with itself: a second one after the bound is a syntax error,
-- not a range over the first one's result.
-- @test
-- @expect-error: syntax error at or near "BETWEEN" (42601)
SELECT 1 BETWEEN 0 AND 2 BETWEEN false AND true;
