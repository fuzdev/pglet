-- A sign in a `FETCH` count takes a numeric constant only; `-n` is a syntax error.
-- @expect-error: syntax error at or near "n"
-- @test
WITH RECURSIVE r(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM r WHERE n < 3 FETCH FIRST -n ROWS ONLY)
SELECT * FROM r;
