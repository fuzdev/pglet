-- `BETWEEN` is `%nonassoc` with `LIKE`.
-- @test
-- @expect-error: syntax error at or near "LIKE" (42601)
SELECT 1 BETWEEN 0 AND 2 LIKE 't';
