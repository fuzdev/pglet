-- @no-txn
-- Drives its own BEGIN/ROLLBACK, so it opts out of the harness's isolating
-- transaction wrapper (and keeps its own DROP-first cleanup).
-- @setup
DROP TABLE IF EXISTS rollback_test CASCADE;
CREATE TABLE rollback_test (id INT, name TEXT);
INSERT INTO rollback_test VALUES (1, 'Alice');
BEGIN;
INSERT INTO rollback_test VALUES (2, 'Bob');
DELETE FROM rollback_test WHERE id = 1;
ROLLBACK;

-- @test
SELECT * FROM rollback_test ORDER BY id;
