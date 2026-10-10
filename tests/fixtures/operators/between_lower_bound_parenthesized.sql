-- The control: a parenthesized lower bound is a full `a_expr`, and a `b_expr` keeps its
-- arithmetic.
-- @test
SELECT 1 BETWEEN (0 IS NOT NULL)::int AND 2 AS a, 2 BETWEEN 1 + 0 AND 3 AS b;
