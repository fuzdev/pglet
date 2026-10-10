-- Inside a transaction block, a probe sees the block's own uncommitted writes: a row the
-- block inserted is found by its key, one it moved is found under the new key, and one it
-- deleted is not found.
-- @profile-lock
-- @no-txn
-- @setup
DROP TABLE IF EXISTS dml_probe_bw;
CREATE TABLE dml_probe_bw (k INT PRIMARY KEY, v TEXT);
INSERT INTO dml_probe_bw VALUES (1, 'a'), (2, 'b');
-- @test
BEGIN;
INSERT INTO dml_probe_bw VALUES (50, 'n');
UPDATE dml_probe_bw SET v = 'u' WHERE k = 50;
UPDATE dml_probe_bw SET k = 60 WHERE k = 50;
DELETE FROM dml_probe_bw WHERE k = 2;
UPDATE dml_probe_bw SET v = 'gone' WHERE k = 2;
UPDATE dml_probe_bw SET v = v || '!' WHERE k = 60 OR k = 1;
COMMIT;
UPDATE dml_probe_bw SET v = v || '?' WHERE k >= 1 RETURNING k, v;
