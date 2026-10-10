-- The `set_config` arm, which builds an `Expr::Alias` around its second argument and so
-- is a third structural shape again. 42809 on both; pglet answered `y`.
-- @test
-- @expect-error: DISTINCT specified, but set_config is not an aggregate function
SELECT set_config(DISTINCT 'x', 'y', false);
