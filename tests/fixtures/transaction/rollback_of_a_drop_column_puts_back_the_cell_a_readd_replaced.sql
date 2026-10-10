-- @no-txn
-- Drives its own transactions, so it opts out of the harness's isolating wrapper
-- (and keeps its own DROP-first cleanup).
--
-- `ALTER TABLE … DROP COLUMN` then `ADD COLUMN … SERIAL` under the same name: the new
-- column's cell is filled by the backfill of the table's rows. With a row deleted the
-- backfill leaves it at `2` while the old cell holds `3`, so keeping the new cell
-- re-issues `3`; 18.6 answers `4`. (Over an undeleted table the two coincide, and a
-- wrong restore passes.)
-- @setup
DROP TABLE IF EXISTS rdc_readd;
CREATE TABLE rdc_readd (x INT, id SERIAL);
INSERT INTO rdc_readd (x) VALUES (1), (2), (3);
DELETE FROM rdc_readd WHERE x = 2;
BEGIN;
ALTER TABLE rdc_readd DROP COLUMN id;
ALTER TABLE rdc_readd ADD COLUMN id SERIAL;
ROLLBACK;
-- @test
INSERT INTO rdc_readd (x) VALUES (4) RETURNING id;
