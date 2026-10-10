-- `coalesce` is a `func_expr_common_subexpr` production on 18.6, not a `pg_proc` entry,
-- so no `func_application` rule can carry a quantifier to it and the refusal is a
-- **syntax** error rather than the 42809 a real function gets. pglet answered `1`.
-- 42601 on both.
-- @test
-- @expect-error: syntax error at or near "DISTINCT"
SELECT coalesce(DISTINCT 1, 2);
