-- The `ON CONFLICT DO UPDATE` half of the `INSERT` clause re-walk, and it needs its own
-- fixture because the action's namespace is a **different object** from `RETURNING`'s:
-- the target plus the `EXCLUDED` pseudo-relation (`on_conflict_action_scope`), where
-- `RETURNING`'s is the target alone (`insert_returning_relations`). One arm cannot serve
-- both.
--
-- **The no-conflict spelling is load-bearing.** `(2, 2)` collides with nothing here, so
-- the action is never evaluated and the eval-time resolver never fires — which is exactly
-- what made this a silent write: pglet inserted `(2, 2)` where 18.6 refuses. Seed a
-- conflicting key instead and the statement reaches eval, which raises on its own, so the
-- fixture would be green with the plan-time gate reverted. (pglet cannot evaluate a
-- subquery inside the action at all — an executed one is `0A000 scalar subquery requires
-- execution context`, a pre-existing gap — so the conflicting spelling could not be
-- written faithfully anyway.)
--
-- `ock_src` stays empty to match its siblings, but note the emptiness is **not** what
-- makes this fixture bite, unlike the `RETURNING` pair where it is: seed `ock_src` and
-- the ungated engine still writes both rows, because the action is never evaluated at
-- all. The no-conflict spelling above is the load-bearing property here.
-- The row this refusal must not write is asserted in
-- `crates/pglet/tests/insert_clause_subplan_binding.rs`.
-- @profile-lock
-- @setup
CREATE TABLE ock_target (id INT PRIMARY KEY, v INT);
CREATE TABLE ock_src (b INT);
INSERT INTO ock_target VALUES (1, 1);

-- @expect-error: column "zzz" does not exist (42703)
-- @test
INSERT INTO ock_target VALUES (2, 2)
  ON CONFLICT (id) DO UPDATE SET v = (SELECT b FROM ock_src ORDER BY zzz);
