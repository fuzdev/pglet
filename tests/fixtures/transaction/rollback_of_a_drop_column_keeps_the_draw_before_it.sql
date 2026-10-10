-- @no-txn
-- Drives its own transactions, so it opts out of the harness's isolating wrapper
-- (and keeps its own DROP-first cleanup).
--
-- `ALTER TABLE … DROP COLUMN … CASCADE` of a `SERIAL` column takes its owned sequence.
-- The block draws `4` in place first, so the old cell goes back at `4` and 18.6 answers
-- `5` after the rollback; restoring the `BEGIN` value re-issues `4`.
-- @setup
DROP TABLE IF EXISTS rdc_draw;
CREATE TABLE rdc_draw (x INT, id SERIAL);
INSERT INTO rdc_draw (x) VALUES (1), (2), (3);
BEGIN;
INSERT INTO rdc_draw (x) VALUES (4) RETURNING id;
ALTER TABLE rdc_draw DROP COLUMN id CASCADE;
ROLLBACK;
-- @test
INSERT INTO rdc_draw (x) VALUES (5) RETURNING id;
