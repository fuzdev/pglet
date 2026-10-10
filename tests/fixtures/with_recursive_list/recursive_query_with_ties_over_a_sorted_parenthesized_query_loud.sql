-- Over a parenthesized query the `ORDER BY` inside it sorts the statement, so `WITH TIES`
-- is grammatical and the form refusal is reached.
-- @expect-error: does not have the form non-recursive-term UNION [ALL] recursive-term
-- @test
WITH RECURSIVE r(n) AS ((SELECT n FROM r ORDER BY 1) FETCH FIRST ROWS WITH TIES)
SELECT * FROM r;
