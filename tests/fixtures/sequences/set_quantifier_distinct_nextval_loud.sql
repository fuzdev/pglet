-- The member of the class that **writes**. `nextval` is an ordinary function on 18.6, so
-- the quantifier refuses the statement in parse analysis and the counter is untouched;
-- pglet dropped the modifier and advanced it, and a sequence advance is
-- non-transactional, so the divergence outlived the statement and any rollback around
-- it. The counter half is guarded by `crates/pglet/tests/set_quantifier_verdicts.rs`,
-- which can observe state after a refusal where a fixture cannot. 42809 on both.
-- @setup
CREATE SEQUENCE sq_nd;
-- @test
-- @expect-error: DISTINCT specified, but nextval is not an aggregate function
SELECT nextval(DISTINCT 'sq_nd');
