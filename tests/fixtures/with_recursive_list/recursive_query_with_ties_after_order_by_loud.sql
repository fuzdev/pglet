-- With an `ORDER BY`, `WITH TIES` is grammatical and the `ORDER BY` refusal is reached.
-- @expect-error: ORDER BY in a recursive query (0A000)
-- @test
WITH RECURSIVE r(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM r WHERE n < 3 ORDER BY n FETCH FIRST ROWS WITH TIES)
SELECT * FROM r;
