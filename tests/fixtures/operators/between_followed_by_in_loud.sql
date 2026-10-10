-- `BETWEEN` is `%nonassoc` with `IN`, `LIKE` and their `NOT` spellings.
-- @test
-- @expect-error: syntax error at or near "NOT" (42601)
SELECT 1 BETWEEN 0 AND 2 NOT IN (true);
