-- A self-referencing body leading with a parenthesized query that repeats a `LIMIT` is
-- refused as the grammar builds it, ahead of the self-reference verdicts.
-- @expect-error: multiple LIMIT clauses not allowed
-- @expect-message
-- @test
WITH RECURSIVE r(n) AS ((SELECT n FROM r LIMIT 1) LIMIT 2) SELECT * FROM r;
