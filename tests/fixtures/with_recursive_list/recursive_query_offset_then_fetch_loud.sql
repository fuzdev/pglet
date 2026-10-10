-- OFFSET then FETCH NEXT is well formed, so the refusal of the clause 18.6 tests first stands.
-- @expect-error: OFFSET in a recursive query (0A000)
-- @expect-message
-- @test
WITH RECURSIVE r(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM r WHERE n < 3 OFFSET 1 ROW FETCH NEXT ROW ONLY)
SELECT * FROM r;
