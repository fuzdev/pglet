-- The identity refusal also outranks pglet's **own** DDL-time over-rejection, the
-- position `set_default_on_identity_beats_missing_sequence.sql` leaves open.
-- `SET DEFAULT nextval('<a table>')` on a plain column is `42809` here
-- (`set_default_nextval_over_a_table_loud.sql` — deliberately stricter than 18.6, which
-- defers the kind check to the first write); on an identity column both engines answer
-- `42601`, because that over-rejection stands in for a *runtime* error and runs last by
-- rule.
--
-- Its twin cannot stand in for it. pglet **auto-creates** a sequence the catalog lacks,
-- so nothing errors ahead of the refusal there and the ordering is unobservable: sinking
-- the gate below the sequence registration (a mutation the sweep tried and found
-- vacuous) leaves `set_default_on_identity_beats_missing_sequence.sql` and every other
-- identity fixture green, and reddens only this one — `SQLSTATE mismatch (Postgres
-- 42601, pglet 42809)`.
-- @setup
CREATE TABLE sdikg_t (id INT GENERATED ALWAYS AS IDENTITY, b INT);
CREATE TABLE sdikg_host (a INT);
-- @test
ALTER TABLE sdikg_t ALTER COLUMN id SET DEFAULT nextval('sdikg_host');
-- @expect-error: the identity refusal outranks pglet's DDL-time 42809 (42601)
