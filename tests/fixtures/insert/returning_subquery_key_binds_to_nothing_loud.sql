-- A `RETURNING` subquery's own binding gate stayed open at its lowering — an expression
-- subquery encloses it, so its level is provisional until an enclosing
-- `resolve_correlated_subplans` re-walk reaches it — and no such re-walk existed for an
-- `INSERT`: `lower_insert` never ran the pass and `walk_level` treated `Plan::Insert` as a
-- leaf. So nobody judged the level, and the statement **wrote its row** and answered NULL
-- where 18.6 is `42703`.
--
-- **`iek_src` must stay empty**, which is the whole reason this fixture pins anything: the
-- eval-time resolver raises the same `42703` per row, so a seeded `iek_src` is green with
-- the gate reverted. A statement whose inner relation has no row evaluates nothing, and
-- the plan-time gate is then the only verdict.
--
-- The `ORDER BY` position is deliberate over a `WHERE` one: a bare sort key is judged
-- against the level's target list first (`findTargetlistEntrySQL92`) and only then the
-- range table, so this reaches the same gate through the claimant-set arm rather than
-- through a plain column lookup. Its qualified twin is
-- `insert/returning_subquery_bogus_qualifier_loud.sql` (`42P01`, the other arm of
-- `check_binding`), and the accept control that this is not closed by over-rejecting is
-- `insert/returning_subquery_names_the_target.sql`.
--
-- The rows this refusal must not write are asserted in
-- `crates/pglet/tests/insert_clause_subplan_binding.rs` — an `@expect-error` compares the
-- SQLSTATE and is green for a refusal raised after the writes.
-- @setup
CREATE TABLE iek_target (v INT);
CREATE TABLE iek_src (b INT);

-- @expect-error: column "zzz" does not exist (42703)
-- @test
INSERT INTO iek_target VALUES (1) RETURNING (SELECT b FROM iek_src ORDER BY zzz);
