-- The alias-ambiguity check must not fire when a **FROM column** claims the name.
-- Postgres's GROUP BY precheck runs first: if `colNameToVar` finds a FROM-clause column
-- named `a`, the targetlist alias search is skipped entirely, so the two entries
-- aliased `a` are never compared and the key binds to `gbafc.a`. The query then fails
-- on the ungrouped `b`: `42803`, not `42702`.
--
-- This pins the *ordering* of the two rules rather than either rule alone — reversing
-- them (checking alias ambiguity before the FROM-column precheck) turns this into
-- `42702` and diverges. See `group_by_alias_ambiguous.sql` for the case where no FROM
-- column claims the name and `42702` is correct.
-- @setup
CREATE TABLE gbafc (a INT, b INT, c INT);
INSERT INTO gbafc VALUES (1, 2, 3);
-- @test
-- @expect-error: column "gbafc.b" must appear in the GROUP BY clause (42803)
SELECT b AS a, c AS a FROM gbafc GROUP BY a;
