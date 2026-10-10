-- @no-txn
-- Drives its own START TRANSACTION/COMMIT, so it opts out of the harness's
-- isolating transaction wrapper (and keeps its own DROP-first cleanup).
-- @setup
DROP TABLE IF EXISTS start_txn_test CASCADE;
CREATE TABLE start_txn_test (id INT, name TEXT);
START TRANSACTION;
INSERT INTO start_txn_test VALUES (1, 'Alice');
INSERT INTO start_txn_test VALUES (2, 'Bob');
COMMIT;

-- @test
SELECT * FROM start_txn_test ORDER BY id;
