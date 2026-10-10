-- An unaliased postfix null test is named `?column?`, as `IS NOT NULL` is; the word is
-- the operator, not the column's label.
-- @test
SELECT 1 NOTNULL;
