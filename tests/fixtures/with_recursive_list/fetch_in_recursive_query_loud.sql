-- A FETCH (LIMIT) over a recursive query is 18.6's `0A000`, raised before either term is
-- examined; it used to stop the parse with `42601`.
-- @expect-error: FETCH (LIMIT) in a recursive query (0A000)
-- @expect-message
-- @test
WITH RECURSIVE r(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM r WHERE n < 3 FETCH FIRST 2 ROWS ONLY)
SELECT * FROM r;
