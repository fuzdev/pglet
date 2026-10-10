-- @no-txn
-- Drives its own BEGIN/ROLLBACK, so it opts out of the harness's isolating wrapper.
--
-- `default_transaction_isolation` is transactional too: a `SET` inside a rolled-back block
-- is taken back, and a `BEGIN` after it runs at the level in force before the block.
-- @setup
DROP TABLE IF EXISTS dti_seen;
CREATE TABLE dti_seen (v TEXT);
SET default_transaction_isolation = 'read committed';
BEGIN;
SET default_transaction_isolation = 'serializable';
ROLLBACK;
BEGIN;
INSERT INTO dti_seen SELECT current_setting('transaction_isolation');
COMMIT;
-- @test
SELECT v FROM dti_seen;
