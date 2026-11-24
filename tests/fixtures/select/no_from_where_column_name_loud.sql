-- With no FROM the `WHERE` reads a row of no columns: a select-list alias is not one.
-- @expect-error: no column x
-- @test
SELECT 1 AS x WHERE x = 1;
