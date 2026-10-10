-- The mirror of `update/assignment_cast_reports_the_first_target_over_a_later_name.sql`,
-- and the one that says the target-name check must sit **beside** the type verdict in
-- the lowering rather than anywhere else. `transformUpdateTargetList` resolves each
-- target's name and coerces that assignment in the same loop iteration, so which of the
-- two errors 18.6 reports is decided purely by written order — `42703` here, `42804`
-- there.
--
-- Delete the plan-time name check and the whole corpus stays green except this file:
-- the gate then fires on the *second* target's `42804` in front of the first target's
-- missing name, because pglet's other name check lives in the executor, downstream of
-- every plan-time verdict. Measured as that mutation.
-- @setup
CREATE TABLE acuy (id INT, b BOOL);

-- @expect-error: column "nosuch" of relation "acuy" does not exist (42703)
-- @test
UPDATE acuy SET nosuch = 1, b = 1;
