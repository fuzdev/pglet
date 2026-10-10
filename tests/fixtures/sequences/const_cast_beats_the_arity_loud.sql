-- Where `planner::transform_call_args` sits against the dispatch. The statement is wrong
-- on two counts — a constant cast that cannot coerce and an argument count `setval` has
-- no signature for — and 18.6 answers the cast, because it transforms the arguments
-- before `func_get_detail` resolves anything. `SELECT setval(DISTINCT 'cca_s', 1, 2, 3)`,
-- the same arity with no cast, is 42883 on both engines and is the control that says
-- which of the two decides; pglet answered 42883 to both until the transform was hoisted
-- above `dispatch_call`.
-- The quantifier is what makes this pinnable, and that is the transform's contract rather
-- than an accident of spelling: it runs for a **quantified** call only, because pglet
-- raises it in its parser, ahead of every executor-time statement gate. The unquantified
-- twin `SELECT setval('cca_s'::int, 1, 2, 3)` is 22P02 on 18.6 and 42883 here — the
-- arity check answers first, exactly as before the quantifier gate — and that divergence is recorded
-- rather than pinned. 22P02 on both.
-- @setup
CREATE SEQUENCE cca_s;
-- @test
-- @expect-error: invalid input syntax for type integer: "cca_s"
SELECT setval(DISTINCT 'cca_s'::int, 1, 2, 3);
