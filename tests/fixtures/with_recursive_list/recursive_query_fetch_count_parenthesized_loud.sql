-- A parenthesized expression is a `FETCH` count and a signed constant an `OFFSET … ROWS`
-- value, so the clauses pass their grammar and the decoration refusal is reached.
-- @expect-error: OFFSET in a recursive query (0A000), tested ahead of LIMIT
-- @test
WITH RECURSIVE r(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM r WHERE n < 3 FETCH FIRST (1 + 1) ROWS ONLY OFFSET -1 ROWS)
SELECT * FROM r;
