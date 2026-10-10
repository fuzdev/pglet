-- @no-txn
-- Drives its own BEGIN/ROLLBACK, so it opts out of the harness's isolating wrapper.
--
-- A restart of an **owned** sequence is undone by its ROLLBACK: `TRUNCATE … RESTART
-- IDENTITY` replaces the owned sequences' cells, and `Plan::swapped_sequences` records
-- each one's value before the statement, which the rollback puts back — the next value
-- is 9, where a restart that outlived its ROLLBACK would answer 7. Postgres gets its
-- answer differently — `ResetSequence` rewrites the sequence relation inside the
-- transaction, so the rollback discards it — but the observable is the same.
--
-- `START WITH 7` is load-bearing: with the default start the restarted counter and
-- the rolled-back one both hand out small numbers and a fixture reading only the
-- first value cannot tell 1-from-a-restart apart from 1-from-a-fresh-sequence.
-- @setup
CREATE TABLE rtri_t (id INT GENERATED ALWAYS AS IDENTITY (START WITH 7), v INT);
INSERT INTO rtri_t (v) VALUES (1), (2);
BEGIN;
TRUNCATE rtri_t RESTART IDENTITY;
ROLLBACK;
INSERT INTO rtri_t (v) VALUES (3);
-- @test
SELECT id, v FROM rtri_t ORDER BY id;
