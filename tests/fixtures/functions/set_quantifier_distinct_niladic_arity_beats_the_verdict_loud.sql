-- The same precedence at the niladic family, whose gate is a different helper
-- (`niladic_call`) reached before the rest of the dispatch: a non-empty argument list
-- fails resolution ahead of the quantifier there too. 42883 on both, and unchanged by the quantifier gate —
-- which is what makes it the control for the arity-first rule at the one
-- family that had an argument gate before it.
-- @test
-- @expect-error: function now(integer) does not exist
SELECT now(DISTINCT 1);
