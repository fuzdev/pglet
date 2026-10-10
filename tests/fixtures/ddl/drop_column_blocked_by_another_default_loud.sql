-- The column route carries the transitive refusal too — the drop would take
-- `dcbad_owner_a_seq` with it, and another table's default draws from it.
-- @setup
CREATE TABLE dcbad_owner (a SERIAL, b INT);
CREATE TABLE dcbad_user (x INT DEFAULT nextval('dcbad_owner_a_seq'));
-- @test
ALTER TABLE dcbad_owner DROP COLUMN a;
-- @expect-error: another table's default depends on the owned sequence (2BP01)
