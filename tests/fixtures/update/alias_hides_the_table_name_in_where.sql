-- **Green before the change that added it, and that is its whole job.** Giving an
-- unaliased UPDATE's scope an identity has one obvious wrong shape — pass the *table*
-- name always — and every reject-polarity fixture in this family stays green under it.
-- This one does not: an alias **hides** the table name (Postgres's `eref`), so once `hw`
-- is aliased `x`, the qualifier `hw` names nothing and the statement is `42P01`. Under
-- "always pass the table" it resolves and the UPDATE succeeds.
--
-- The right rule is the alias when one is written, else the table's own name — one
-- value, computed once in `execute_update` and handed to both collection paths.
--
-- The accept polarity of the same rule is the ordinary `UPDATE hw SET … WHERE hw.id = 1`
-- and `UPDATE hw x SET … WHERE x.id = 1`, which the corpus already exercises widely.
-- `update/returning_alias_hides_table_name.sql` pins the same Postgres rule in the
-- `RETURNING` clause, whose namespace is built by a different derivation.
--
-- **This fixture pins one of the rule's two paths, and the other used to be a write.**
-- The reference here reaches `SimpleColumns`, whose tier 3 refuses a qualifier its scope
-- does not name; a *correlated* one reaches the outer-frame chain, which used to fall to
-- a bare match that never read the qualifier. Over `hw(id, v)` = `(1,'hit'),(2,'miss')`
-- and a one-row `u2b(yv)` = `('hit')`, `UPDATE hw x SET id = 99 WHERE EXISTS (SELECT 1
-- FROM u2b WHERE u2b.yv = hw.v)` was `UPDATE 1`, writing `99` over the `hit` row, where
-- 18.6 is `42P01 "hw"` with a HINT naming the alias `x`. That path is closed and pinned
-- by `update/correlated_outer_ref_alias_hides_the_table_name.sql`, with the surviving
-- rows asserted in `crates/pglet/tests/scope_relation_identity.rs` — `@expect-error` is
-- green for a refusal raised after the writes, which is what the two guards split.
-- @setup
CREATE TABLE hw (id INT, v TEXT);
INSERT INTO hw VALUES (1, 'a'), (2, 'b');

-- @expect-error: invalid reference to FROM-clause entry for table "hw" (42P01)
-- @test
UPDATE hw x SET v = 'z' WHERE hw.id = 1;
