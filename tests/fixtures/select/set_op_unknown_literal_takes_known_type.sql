-- A set operation resolves each column with `select_common_type`, where a bare string
-- constant is `unknown` and contributes no type: it takes the other side's, whichever
-- side it is on. Column `a` has the unknown on the right, column `b` on the left; both
-- are `integer`. Reading the constant as `text` refuses the pair `42804`.
-- @test
SELECT 1 AS a, '2' AS b UNION ALL SELECT '3', 4 ORDER BY 1;
