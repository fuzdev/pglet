-- @no-txn
-- Drives its own BEGIN/ROLLBACK, so it opts out of the harness's isolating wrapper.
--
-- The polarity of `rollback_truncate_restart_identity.sql`. A **plain** TRUNCATE
-- replaces no sequence cell, so `Plan::swapped_sequences` lists none and the rollback
-- carries the live counter forward, keeping an unrelated `nextval` issued in the same
-- transaction, as 18.6 does. A rollback that restored every counter from its frame, not
-- only the cells that list names, would revert that advance and turn this fixture red
-- (measured through the embed facade: `1|2`), and so would a TRUNCATE reading that lists
-- sequences the truncated table does not own.
-- @setup
CREATE TABLE rptk_a (v INT);
CREATE TABLE rptk_b (id INT GENERATED ALWAYS AS IDENTITY, v INT);
BEGIN;
TRUNCATE rptk_a;
INSERT INTO rptk_b (v) VALUES (1);
ROLLBACK;
INSERT INTO rptk_b (v) VALUES (2);
-- @test
SELECT id, v FROM rptk_b ORDER BY id;
