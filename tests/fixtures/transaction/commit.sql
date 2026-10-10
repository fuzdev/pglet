-- @no-txn
-- Drives its own BEGIN/COMMIT, so it opts out of the harness's isolating
-- transaction wrapper (and keeps its own DROP-first cleanup).
-- @setup
DROP TABLE IF EXISTS txn_test CASCADE;
CREATE TABLE txn_test (id INT, name TEXT);
BEGIN;
INSERT INTO txn_test VALUES (1, 'Alice');
INSERT INTO txn_test VALUES (2, 'Bob');
COMMIT;

-- @test
SELECT * FROM txn_test ORDER BY id;
