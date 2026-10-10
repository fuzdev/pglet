-- Outside a block, a probe reads the committed index: each autocommitted write is found by
-- the next statement's probe.
-- @profile-lock
-- @no-txn
-- @setup
DROP TABLE IF EXISTS dml_probe_ac;
CREATE TABLE dml_probe_ac (k INT PRIMARY KEY, v TEXT);
INSERT INTO dml_probe_ac VALUES (1, 'a'), (2, 'b'), (3, 'c');
-- @test
UPDATE dml_probe_ac SET k = 30 WHERE k = 3;
DELETE FROM dml_probe_ac WHERE k = 1;
UPDATE dml_probe_ac SET v = v || '!' WHERE k = 30;
DELETE FROM dml_probe_ac WHERE k = 30 RETURNING k, v;
