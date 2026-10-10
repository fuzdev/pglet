-- `WITH TIES` beside `SKIP LOCKED` is refused as the grammar builds the statement,
-- `42601` ahead of the `ORDER BY` refusal.
-- @expect-error: SKIP LOCKED and WITH TIES options cannot be used together
-- @expect-message
-- @test
WITH RECURSIVE r(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM r WHERE n < 3 ORDER BY n FETCH FIRST ROWS WITH TIES FOR UPDATE SKIP LOCKED)
SELECT * FROM r;
