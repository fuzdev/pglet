-- An **expression** unique index: pglet can name neither the key column nor the type its
-- value renders under, so it omits the `D` 18.6 sends
-- (`Key (lower(a))=(a) already exists.`) — and sends `s`/`t`/`n` anyway, because the
-- relation is truthful whatever the key looks like. What this fixture pins is the
-- *invention* half: pglet must not start sending a `D` it cannot derive, and the `s`/`t`/`n`
-- it does send must match. It cannot pin the omission half — a missing field scores as
-- agreement here — so dropping `t` with the key description is caught by
-- `pglet_core::error_fields`'s `an_expression_index_conflict_keeps_the_table_and_drops_only_the_detail`,
-- not by this file.
-- @no-profile-lock: the 23505 is raised by `enforce_index_entries` on the INSERT's write path — `executor/constraints.rs` carries no `index_acceleration` cfg and the INSERT plans no predicate
-- @setup
CREATE TABLE ex_u (a TEXT);
CREATE UNIQUE INDEX ex_u_i ON ex_u (lower(a));
INSERT INTO ex_u VALUES ('a');

-- @test
-- @expect-error: the expression key collides case-insensitively
-- @expect-message
INSERT INTO ex_u VALUES ('A');
