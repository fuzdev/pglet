-- @no-txn
-- Drives its own transactions, so it opts out of the harness's isolating wrapper
-- (and keeps its own DROP-first cleanup).
--
-- A sequence the dropped table's `DEFAULT` names but does not own survives the drop, so
-- the drop replaces no cell of it and its in-place draws stand after the rollback: the
-- block draws `4` after the drop, and 18.6 answers `5`. Reading the dropped table's
-- defaults, rather than the owner edge, lists this sequence and answers `4` again.
-- @setup
DROP TABLE IF EXISTS rdt_ref;
DROP SEQUENCE IF EXISTS rdt_ref_s;
CREATE SEQUENCE rdt_ref_s;
CREATE TABLE rdt_ref (x INT DEFAULT nextval('rdt_ref_s'));
INSERT INTO rdt_ref DEFAULT VALUES;
INSERT INTO rdt_ref DEFAULT VALUES;
INSERT INTO rdt_ref DEFAULT VALUES;
BEGIN;
DROP TABLE rdt_ref;
SELECT nextval('rdt_ref_s');
ROLLBACK;
-- @test
SELECT nextval('rdt_ref_s');
