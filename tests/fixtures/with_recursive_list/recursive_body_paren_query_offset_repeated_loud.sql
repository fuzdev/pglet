-- A self-referencing body leading with a parenthesized query that repeats an `OFFSET` is
-- refused as the grammar builds it.
-- @expect-error: multiple OFFSET clauses not allowed
-- @expect-message
-- @test
WITH RECURSIVE r(n) AS ((SELECT n FROM r OFFSET 1) OFFSET 1) SELECT * FROM r;
