-- The refusal is not "the edge would change" — it is "the edge is INTERNAL". Naming
-- the identity column the sequence already belongs to is a no-op ownership-wise and
-- is still 0A000 on 18.6, because `process_owned_by` tests the sequence's existing
-- INTERNAL dependency before it looks at what the new edge would be. A gate written
-- as "refuse a *different* owner" passes every other fixture here and fails this one.
-- @setup
CREATE TABLE aobsame_t (id INT GENERATED ALWAYS AS IDENTITY, b INT);
-- @test
ALTER SEQUENCE aobsame_t_id_seq OWNED BY aobsame_t.id;
-- @expect-error: cannot change ownership of identity sequence (0A000)
