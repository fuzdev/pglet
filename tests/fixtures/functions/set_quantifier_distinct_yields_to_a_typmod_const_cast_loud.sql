-- A **typmod'd** constant cast is a constant cast: 18.6's argument transform coerces
-- `'abc'` with `numeric`'s input function, which is the same `stringTypeDatum` call the
-- plain `'abc'::numeric` spelling reaches, so both are 22P02 ahead of the quantifier.
-- pglet builds two different nodes for the two spellings (`Expr::CastTypmod` carries the
-- modifier so eval can enforce it) and the transform matched only the plain one, so this
-- answered 42809 while its cast-free control agreed — one rule, two answers, decided by
-- whether the target was written with a modifier. 22P02 on both.
-- @test
-- @expect-error: invalid input syntax for type numeric: "abc"
SELECT concat(DISTINCT 'abc'::numeric(5,2));
