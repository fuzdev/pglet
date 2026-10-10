-- The refused row's cells are transformed before its length is compared, but its
-- assignment to the target columns — where an unknown literal meets its column's input
-- function — comes after, so the short row's `'x'` never reaches `integer`: `42601`.
-- @setup
CREATE TABLE vlrl (a INT, b INT);

-- @test
-- @expect-error: VALUES lists must all be the same length (42601), ahead of the short row's own literal
-- @expect-message
INSERT INTO vlrl VALUES (1, 2), ('x');
