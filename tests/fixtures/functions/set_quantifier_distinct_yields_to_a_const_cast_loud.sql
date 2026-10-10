-- 18.6 transforms a call's arguments (`transformExprRecurse`) before `ParseFuncOrColumn`
-- judges the call, so a constant cast — coerced during that transform — outranks the set
-- quantifier for **every** name, not just the sequence trio whose arm discovered the rule.
-- `planner::transform_call_args` runs it above the dispatch for that reason. Written on
-- the sequence arm alone, this statement answered 42809 here against 22P02 there, while
-- `sequences/set_quantifier_distinct_yields_to_a_const_cast_loud.sql` agreed — one rule,
-- two answers, decided by the name. 22P02 on both.
-- @test
-- @expect-error: invalid input syntax for type integer: "a"
SELECT upper(DISTINCT 'a'::int);
