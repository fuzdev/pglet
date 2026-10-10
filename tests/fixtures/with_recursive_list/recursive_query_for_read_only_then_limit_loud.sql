-- A `LIMIT` after `FOR READ ONLY` is the `LIMIT` refusal.
-- @expect-error: LIMIT in a recursive query (0A000)
-- @expect-message
-- @test
WITH RECURSIVE r(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM r WHERE n < 3 FOR READ ONLY LIMIT 2) SELECT * FROM r;
