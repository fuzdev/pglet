-- The headline of the class: a set quantifier is aggregate-only, so
-- `ParseFuncOrColumn`'s `agg_distinct` arm refuses it for anything that resolves to an
-- ordinary function — the `if` immediately below the `agg_star` arm the niladic-arity
-- close implemented. pglet parsed the modifier and dropped it on the floor at every
-- non-aggregate arm, answering `A` here — and `build_call` is the one site a function
-- call is built at, so that reached wherever an expression is parsed. 42809 on both.
-- @test
-- @expect-error: DISTINCT specified, but upper is not an aggregate function
SELECT upper(DISTINCT 'a');
