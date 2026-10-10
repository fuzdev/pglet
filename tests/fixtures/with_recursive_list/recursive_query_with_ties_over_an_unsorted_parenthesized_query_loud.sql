-- Over a parenthesized query that sorts nothing, `WITH TIES` has no `ORDER BY` to merge
-- with, and is refused as the grammar builds the statement.
-- @expect-error: WITH TIES cannot be specified without ORDER BY clause
-- @expect-message
-- @test
WITH RECURSIVE r(n) AS ((SELECT n FROM r) FETCH FIRST 1 ROWS WITH TIES) SELECT * FROM r;
