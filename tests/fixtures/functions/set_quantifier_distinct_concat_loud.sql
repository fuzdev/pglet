-- The non-strict variadic arm (`Expr::VariadicFn`). `concat` range-checks nothing at
-- all, so nothing in this arm could have answered ahead of the quantifier. 42809 on
-- both; pglet answered `a`.
-- @test
-- @expect-error: DISTINCT specified, but concat is not an aggregate function
SELECT concat(DISTINCT 'a');
