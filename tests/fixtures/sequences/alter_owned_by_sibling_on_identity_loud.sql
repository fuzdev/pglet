-- ...and when the edge is moved to a plain sibling column of the identity's own
-- table. `ALTER TABLE … DROP COLUMN <the sibling>` then takes the sequence, so this
-- reaches the same state as the other-table route without a second relation.
-- @setup
CREATE TABLE aobsib_t (id INT GENERATED ALWAYS AS IDENTITY, b INT);
-- @test
ALTER SEQUENCE aobsib_t_id_seq OWNED BY aobsib_t.b;
-- @expect-error: cannot change ownership of identity sequence (0A000)
