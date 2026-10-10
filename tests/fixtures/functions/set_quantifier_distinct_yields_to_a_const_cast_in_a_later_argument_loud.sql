-- The transform is over the **whole** argument list and this is the spelling that says so:
-- the cast is on the second argument, and the first is fine. A gate reading only the
-- first argument passes this and answers the quantifier's 42809; a gate reading none
-- answers it too. `concat` also puts the rule on a non-string arm (`Expr::VariadicFn`) —
-- and on the one variadic arm that range-checks nothing at all, so the 42809 really is
-- what this statement answered without the transform. 22P02 on both.
-- @test
-- @expect-error: invalid input syntax for type integer: "b"
SELECT concat(DISTINCT 'a', 'b'::int);
