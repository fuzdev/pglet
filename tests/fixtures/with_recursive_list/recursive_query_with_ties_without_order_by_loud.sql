-- `WITH TIES` without an `ORDER BY` is refused as the grammar builds the statement
-- (`insertSelectOptions`), `42601` ahead of the `FETCH` refusal.
-- @expect-error: WITH TIES cannot be specified without ORDER BY clause
-- @expect-message
-- @test
WITH RECURSIVE r(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM r WHERE n < 3 FETCH FIRST ROWS WITH TIES)
SELECT * FROM r;
