-- Without `ROWS` an `OFFSET` takes any expression, so the `OFFSET` refusal is reached.
-- @expect-error: OFFSET in a recursive query (0A000)
-- @test
WITH RECURSIVE r(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM r WHERE n < 3 OFFSET 1 + 1)
SELECT * FROM r;
