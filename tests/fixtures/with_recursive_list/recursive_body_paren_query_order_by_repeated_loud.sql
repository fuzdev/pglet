-- A self-referencing body leading with a parenthesized query that repeats an `ORDER BY`
-- is refused as the grammar builds it.
-- @expect-error: multiple ORDER BY clauses not allowed
-- @expect-message
-- @test
WITH RECURSIVE r(n) AS ((SELECT n FROM r ORDER BY n) ORDER BY n) SELECT * FROM r;
