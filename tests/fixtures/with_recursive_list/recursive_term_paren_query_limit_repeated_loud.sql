-- A parenthesized recursive term repeating a clause of its own inner query is refused.
-- @expect-error: multiple LIMIT clauses not allowed
-- @expect-message
-- @test
WITH RECURSIVE r(n) AS (SELECT 1 UNION ALL ((SELECT n + 1 FROM r WHERE n < 3 LIMIT 1) LIMIT 2)) SELECT * FROM r;
