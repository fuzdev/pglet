-- FETCH FIRST is well formed, so the refusal of the clause 18.6 tests first stands.
-- @expect-error: LIMIT in a recursive query (0A000)
-- @expect-message
-- @test
WITH RECURSIVE r(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM r WHERE n < 3 FETCH FIRST 2 ROWS ONLY)
SELECT * FROM r;
