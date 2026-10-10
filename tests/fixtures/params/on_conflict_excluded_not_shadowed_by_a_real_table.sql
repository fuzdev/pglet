-- `excluded` is a **pseudo-relation of the action**, not a name looked up in the
-- catalog — and the only statement that can tell those apart is one whose catalog
-- holds a real table called `excluded`. This is that statement.
--
-- The shadow table carries the target's column names under different types (`n TEXT`
-- against the target's `n BIGINT`), so a walk that resolves `excluded.n` through the
-- catalog types `$1` as text and one that resolves it through the action's own
-- namespace types it bigint. Measured: with `excluded.<col>` typed off
-- `TypeScope::Catalog`'s no-such-table fallback — right until such a table exists —
-- pglet reported `{text}` where 18.6 reports `{bigint}`, and this bound statement
-- was then `42883 operator does not exist: bigint = text` while 18.6 updated the row.
-- A false rejection produced by nothing but an ordinary table name.
--
-- The bound value discriminates the **resolution** as well as the type, exactly as in
-- `on_conflict_excluded_qualified_bound.sql`: `excluded.n` is the would-be-inserted
-- `99` while the stored row's `n` is `10`, so a predicate reading the stored row
-- returns no rows at all. Keep the shadow table's `n` non-numeric — give it `BIGINT`
-- and both resolutions report the same OID and the fixture stops discriminating.
--
-- The arbiter is the other half of the rule and is **not** testable here: on 18.6
-- `ON CONFLICT ((excluded.id)) …` and `ON CONFLICT (id) WHERE excluded.n > 0 …` are
-- `42P01 invalid reference to FROM-clause entry for table "excluded"`, which pglet
-- does not raise because it resolves no arbiter names at all — so that pair is a
-- recorded divergence (`docs/compatibility.md`) with a unit-test guard
-- (`param_types.rs`'s `an_arbiter_reference_is_not_given_the_actions_namespace`),
-- not a green fixture.
-- @setup
CREATE TABLE oc_shadow (id INT PRIMARY KEY, v TEXT, n BIGINT);
CREATE TABLE excluded (id INT, n TEXT, v TEXT);
INSERT INTO oc_shadow VALUES (1, 'old', 10);
-- @param $1 int8 99
-- @test
INSERT INTO oc_shadow (id, v, n) VALUES (1, 'new', 99) ON CONFLICT (id) DO UPDATE SET v = 'z' WHERE excluded.n = $1 RETURNING id, v, n;
