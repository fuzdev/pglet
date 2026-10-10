-- The `42P01` arm of the gate `insert/returning_subquery_key_binds_to_nothing_loud.sql`
-- pins at `42703`: a qualifier the `RETURNING` namespace does not name.
-- `ColumnScope::check_binding` answers `NoRelation` and `NoColumn` from different tiers,
-- so the `42703` fixture alone does not cover this one — and before the `INSERT` clauses
-- got their re-walk this spelling **wrote its row** too, answering NULL.
--
-- `ibq_src` stays empty for the reason its sibling states: with a row, the eval-time
-- resolver raises this `42P01` itself and the fixture is green with the gate reverted.
-- @setup
CREATE TABLE ibq_target (v INT);
CREATE TABLE ibq_src (b INT);

-- @expect-error: missing FROM-clause entry for table "zzz" (42P01)
-- @test
INSERT INTO ibq_target VALUES (1) RETURNING (SELECT b FROM ibq_src WHERE zzz.q = 1);
