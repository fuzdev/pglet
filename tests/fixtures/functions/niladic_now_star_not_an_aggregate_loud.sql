-- A wildcard argument list parses to an **empty** `args`, so an arity-only guard passes
-- it through — the two checks are not one. 18.6 reaches a different verdict for it too:
-- `func_get_detail` resolves the signature first and a `*` contributes no argument, so
-- a name carrying a zero-argument entry lands on `ParseFuncOrColumn`'s `agg_star` arm
-- (42809) rather than on the 42883 a non-empty list gets.
-- @test
-- @expect-error: now(*) specified, but now is not an aggregate function
SELECT now(*);
