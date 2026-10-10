-- @no-txn
-- Drives its own BEGIN/ROLLBACK, so it opts out of the harness's isolating
-- transaction wrapper.
--
-- The mirror of `rollback_create_sequence.sql`: a block whose one statement drops a
-- sequence takes the drop back at its ROLLBACK, which restores the catalog the block
-- took the writer on, and `Plan::swapped_sequences` puts back the counter the dropped
-- cell held. The transaction is kept to exactly one statement so no other DDL can
-- carry the restore.
-- @setup
CREATE SEQUENCE rbds_seq START WITH 40;
BEGIN;
DROP SEQUENCE rbds_seq;
ROLLBACK;
-- @test
SELECT nextval('rbds_seq');
