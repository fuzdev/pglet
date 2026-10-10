-- 23505 carries `t` (which the constraint name alone does not determine) and a
-- `Key (…)=(…) already exists.` DETAIL.
-- @setup
CREATE TABLE t_u (id INT PRIMARY KEY, v TEXT);
INSERT INTO t_u VALUES (1, 'x');

-- @test
-- @expect-error: duplicate primary key
-- @no-profile-lock: the 23505 is raised by `enforce_index_entries` on the INSERT's own write path, not by index selection — `executor/constraints.rs` carries no `index_acceleration` cfg, so the probe and its error are the same code on both profiles, and the `@test` has no predicate to plan
-- @expect-message
INSERT INTO t_u VALUES (1, 'y');
