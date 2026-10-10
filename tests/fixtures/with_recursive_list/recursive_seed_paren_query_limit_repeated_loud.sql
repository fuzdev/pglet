-- A parenthesized non-recursive term repeating a clause of its own inner query is refused,
-- although a set operation follows it.
-- @expect-error: multiple LIMIT clauses not allowed
-- @expect-message
-- @test
WITH RECURSIVE r(n) AS (((SELECT n FROM r LIMIT 1) LIMIT 2) UNION ALL SELECT n + 1 FROM r WHERE n < 3) SELECT * FROM r;
