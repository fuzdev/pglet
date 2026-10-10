-- The inner `LIMIT` of a query parenthesized twice reaches the `LIMIT` after both.
-- @expect-error: multiple LIMIT clauses not allowed
-- @expect-message
-- @test
WITH RECURSIVE r(n) AS (((SELECT n FROM r LIMIT 1)) LIMIT 2) SELECT * FROM r;
