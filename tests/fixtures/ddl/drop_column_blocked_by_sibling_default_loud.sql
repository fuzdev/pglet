-- The deletion set is **column**-scoped here, so a sibling column of the *same* table
-- blocks — the arrangement neither reading of "self-reference does not block"
-- predicts, and the same catalog that drops cleanly as a whole table.
-- @setup
CREATE TABLE dcsib_t (id SERIAL, b INT DEFAULT nextval('dcsib_t_id_seq'));
-- @test
ALTER TABLE dcsib_t DROP COLUMN id;
-- @expect-error: the sibling column's default depends on the sequence (2BP01)
