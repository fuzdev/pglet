-- The no-FROM `WHERE` is a boolean context, as a FROM query's is.
-- @expect-error: argument of WHERE must be boolean
-- @test
SELECT 1 WHERE 1;
