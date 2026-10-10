-- The **third** clause position the `INSERT` re-walk serves, and the one no other guard
-- in this family touches: the `ON CONFLICT DO UPDATE` action's own `WHERE`. Its namespace
-- is the action's (`on_conflict_action_scope` — the target plus `EXCLUDED`), the same
-- object the `SET` right-hand sides read, but it is a separate visit in
-- `correlation::walk_level`'s `Plan::Insert` arm and deleting that visit alone left the
-- whole fixture corpus and `cargo test --workspace` green while this statement went back
-- to writing its row.
--
-- **No conflict**, so the action is never evaluated and the eval-time resolver never
-- fires — the plan-time gate is the only verdict, which is what made the unguarded
-- version a silent write rather than a loud error. `ocw_src` stays empty for the same
-- reason its `RETURNING` sibling states.
--
-- The row this refusal must not write is asserted in
-- `crates/pglet/tests/insert_clause_subplan_binding.rs`; an `@expect-error` compares the
-- SQLSTATE and is green for a refusal raised after the writes.
-- @profile-lock
-- @setup
CREATE TABLE ocw_target (id INT PRIMARY KEY, v INT);
CREATE TABLE ocw_src (b INT);
INSERT INTO ocw_target VALUES (1, 1);

-- @expect-error: column "zzz" does not exist (42703)
-- @test
INSERT INTO ocw_target VALUES (3, 3)
  ON CONFLICT (id) DO UPDATE SET v = 1 WHERE (SELECT b FROM ocw_src ORDER BY zzz) IS NULL;
