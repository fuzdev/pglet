-- **An error-order control — do not trim it.** The derived-table spelling of
-- `assignment_cast_set_yields_to_a_missing_source.sql`, and the one that says counting
-- registered relations is not the same question as resolving them: a
-- `Plan::SubqueryScan` registers itself happily whatever its body names, so the
-- "did the source register a relation" test passes and only a walk **into** the
-- sub-plan finds the missing one.
--
-- 18.6 is `42P01` here whatever the `SET` list says; pglet raises that at execution, so
-- a plan-time `42804` on the perfectly-typed `1` pre-empts it.
-- @setup
CREATE TABLE acmd (id INT, b BOOL);

-- @expect-error: relation "acmd_nosuch" does not exist (42P01)
-- @test
UPDATE acmd SET b = 1 FROM (SELECT n FROM acmd_nosuch) s;
