-- The identity refusal sits *after* the owner is resolved, not before it:
-- `process_owned_by` opens the named relation, checks its kind and looks up the
-- column, and only then tests the sequence's INTERNAL dependency. So a missing owner
-- table is 42P01 even on an identity sequence, where the gate placed one step earlier
-- would answer 0A000.
-- @setup
CREATE TABLE aobmt_t (id INT GENERATED ALWAYS AS IDENTITY, b INT);
-- @test
ALTER SEQUENCE aobmt_t_id_seq OWNED BY nosuch_aobmt_table.a;
-- @expect-error: the missing owner table outranks the identity refusal (42P01)
