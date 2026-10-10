-- An identity column's default is the engine's link to its own sequence, not the
-- user's to move: `ATExecColumnDefault` raises 42601 for both spellings.
--
-- The other half of the fact `alter_owned_by_none_on_identity_loud.sql` guards.
-- Postgres stores no `pg_attrdef` row for an identity column at all, so refusing
-- costs it nothing; pglet *does* model one, so accepting this severed the link and
-- the sequence — no longer the column's internal implementation — then dropped
-- through an ordinary `DROP SEQUENCE`, leaving the column's default dangling.
-- @setup
CREATE TABLE sdid_t (id INT GENERATED ALWAYS AS IDENTITY, b INT);
CREATE SEQUENCE sdid_other;
-- @test
ALTER TABLE sdid_t ALTER COLUMN id SET DEFAULT nextval('sdid_other');
-- @expect-error: column is an identity column (42601)
