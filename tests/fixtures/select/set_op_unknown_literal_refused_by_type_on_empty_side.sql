-- The unknown constant is converted to the column's type during parse analysis, so a
-- constant the type refuses is `22P02` even where no row is ever produced. Converting
-- it only as rows flow answers zero rows.
-- @setup
CREATE TABLE sue (a INT);
-- @expect-error: invalid input syntax for type integer
-- @expect-message
-- @test
SELECT a FROM sue UNION ALL SELECT 'x' FROM sue;
