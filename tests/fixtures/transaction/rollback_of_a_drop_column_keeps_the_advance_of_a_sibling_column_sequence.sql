-- @no-txn
-- Drives its own transactions, so it opts out of the harness's isolating wrapper
-- (and keeps its own DROP-first cleanup).
--
-- `ALTER TABLE … DROP COLUMN` takes only the sequence the dropped column owns: a
-- sibling `SERIAL` column's sequence survives, so its in-place draw after the drop
-- stands after the rollback — the block draws `4` from it, and 18.6 answers `5`.
-- Listing every sequence the table owns answers `4` again.
-- @setup
DROP TABLE IF EXISTS rdc_sibling;
CREATE TABLE rdc_sibling (a SERIAL, b SERIAL, x INT);
INSERT INTO rdc_sibling (x) VALUES (1), (2), (3);
BEGIN;
ALTER TABLE rdc_sibling DROP COLUMN a;
INSERT INTO rdc_sibling (x) VALUES (4) RETURNING b;
ROLLBACK;
-- @test
INSERT INTO rdc_sibling (x) VALUES (5) RETURNING b;
