-- `OFFSET … ROWS` takes the same value a `FETCH` count does, so `1 + 1 ROWS` is a syntax
-- error.
-- @expect-error: syntax error at or near "ROWS"
-- @test
WITH RECURSIVE r(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM r WHERE n < 3 OFFSET 1 + 1 ROWS)
SELECT * FROM r;
