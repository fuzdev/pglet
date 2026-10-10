-- The temporal arm that builds an `Expr::Extract` — `date_part` is the float8-returning
-- twin of `EXTRACT`, and unlike `EXTRACT` it is an ordinary call rather than a grammar
-- form, so it reaches `build_call` and takes the quantifier there. 42809 on both; pglet
-- answered `2026`. (`extract(DISTINCT year FROM …)` is 42601 on both — the grammar form,
-- refused in each engine's own parser.)
-- @test
-- @expect-error: DISTINCT specified, but date_part is not an aggregate function
SELECT date_part(DISTINCT 'year', now());
