-- An identity column's sequence is its INTERNAL implementation: `cannot drop sequence
-- … because column id of table dsid_t requires it`. The column has to go instead.
-- @setup
CREATE TABLE dsid_t (id INT GENERATED ALWAYS AS IDENTITY, b INT);
-- @test
DROP SEQUENCE dsid_t_id_seq;
-- @expect-error: an identity column requires its sequence (2BP01)
