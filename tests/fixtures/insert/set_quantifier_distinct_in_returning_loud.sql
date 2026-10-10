-- A `RETURNING` position, where the divergence used to be a **written row** as well as a
-- wrong answer: 18.6 refuses the whole statement in parse analysis, so nothing is
-- inserted, while pglet inserted the row and projected `A`. The no-write half is guarded
-- by `crates/pglet/tests/set_quantifier_verdicts.rs`. 42809 on both.
-- @setup
CREATE TABLE sq_rtd (c TEXT);
-- @test
-- @expect-error: DISTINCT specified, but upper is not an aggregate function
INSERT INTO sq_rtd VALUES ('a') RETURNING upper(DISTINCT c);
