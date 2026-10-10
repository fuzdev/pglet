-- @no-txn
-- Drives its own transactions, so it opts out of the harness's isolating wrapper
-- (and keeps its own DROP-first cleanup).
--
-- The owned-sequence routes of
-- `do_completed_in_a_block_hands_its_replaced_cells_to_the_rollback.sql`, each over a
-- serial table holding ids `1`–`3`: a `DO`'s `TRUNCATE … RESTART IDENTITY` in a block
-- that rolls back, and a `DO`'s `DROP TABLE` and re-creation between a `SAVEPOINT` and a
-- `ROLLBACK TO` it, in a block that then commits. The rollback puts each identity cell
-- back as it was: 18.6 answers `4 4`. Handed nothing, it left the replacing cells'
-- `1 1`, and the next insert re-issued an id the table holds.
-- @setup
DROP TABLE IF EXISTS dco_t, dco_w;
CREATE TABLE dco_t (x INT, id SERIAL);
CREATE TABLE dco_w (x INT, id SERIAL);
INSERT INTO dco_t (x) VALUES (1), (2), (3);
INSERT INTO dco_w (x) VALUES (1), (2), (3);
BEGIN;
DO $$ BEGIN TRUNCATE dco_t RESTART IDENTITY; END $$;
ROLLBACK;
BEGIN;
SAVEPOINT a;
DO $$ BEGIN DROP TABLE dco_w; CREATE TABLE dco_w (x INT, id SERIAL); END $$;
ROLLBACK TO SAVEPOINT a;
COMMIT;
-- @test
SELECT nextval('dco_t_id_seq') AS t, nextval('dco_w_id_seq') AS w;
