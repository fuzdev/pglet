-- An identity column's sequence is its INTERNAL implementation, so Postgres refuses
-- to let the ownership edge be cleared at all: `process_owned_by` tests
-- `sequenceIsOwned(seq, DEPENDENCY_INTERNAL, …)` and raises 0A000. Without the
-- refusal the edge detaches, an ordinary `DROP SEQUENCE` then removes the sequence
-- (nothing reports the identity column any more), and the column's `DEFAULT nextval`
-- names a relation the catalog lacks — every write 42P01, and a dump that emits the
-- default with no `CREATE SEQUENCE` for it.
-- @setup
CREATE TABLE aobin_t (id INT GENERATED ALWAYS AS IDENTITY, b INT);
-- @test
ALTER SEQUENCE aobin_t_id_seq OWNED BY NONE;
-- @expect-error: cannot change ownership of identity sequence (0A000)
