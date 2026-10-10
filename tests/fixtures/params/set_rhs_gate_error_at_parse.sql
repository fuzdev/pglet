-- The sibling of `params/returning_gate_error_at_parse.sql` over a different gate
-- and a different clause: the `SET` right-hand side's common-type fold (`42804`)
-- rather than the `RETURNING` projection's operator gate (`42883`), with the bound
-- parameter in the `WHERE` instead.
--
-- Two shapes, not one, because the swallowed-plan-error bug was in the Parse handler
-- and therefore indifferent to *which* clause failed to plan — a lock on a single
-- clause would leave the claim resting on one gate. Postgres raises both at Parse;
-- pglet reported `08P01` for both.
--
-- The parameter is deliberately not in the ill-typed clause: the plan fails for a
-- reason that has nothing to do with `$1`, and the count check still fired, which is
-- what made the old answer wrong rather than approximate.
-- @setup
CREATE TABLE pgate_s (i4 INT4, tx TEXT);
INSERT INTO pgate_s VALUES (7, 'a');
-- @param $1 int4 7
-- @test
-- @expect-error: the SET right-hand side's no-common-type 42804, not Bind's 08P01
UPDATE pgate_s SET tx = COALESCE(i4, tx) WHERE i4 = $1;
