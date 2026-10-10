-- One UPDATE carrying both a target-side NOT NULL violation and a referenced-key change
-- a child would refuse. 18.6 runs `ExecConstraints` on the new tuple inside `ExecUpdate`
-- and queues the referential actions as AFTER-ROW triggers, so the target's `23502`
-- outranks the child's `23001` — the ordering a row-scoped NOT NULL check placed at the
-- write, after the FK plan, silently inverted. No fixture paired the two before this:
-- `fk/on_update_set_null_not_null.sql` puts the NOT NULL on the child.
-- @profile-lock
-- @setup
CREATE TABLE nn_p (id INT PRIMARY KEY, v INT NOT NULL);
CREATE TABLE nn_c (pid INT REFERENCES nn_p(id) ON UPDATE RESTRICT);
INSERT INTO nn_p VALUES (1, 1);
INSERT INTO nn_c VALUES (1);

-- @test
-- @expect-error: the target's own NOT NULL, not the child's RESTRICT
-- @expect-message
UPDATE nn_p SET id = 2, v = NULL;
