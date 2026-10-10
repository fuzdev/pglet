-- A declared `$N` is a known type in a set operation, so a string constant on either
-- side takes it: column `a` has the constant on the right, column `b` on the left.
-- @param-declared $1 int4 5
-- @test
SELECT $1 AS a, '6' AS b UNION ALL SELECT '7', $1;
