-- @no-txn
-- A nested/redundant BEGIN inside a DDL transaction must be a no-op (Postgres
-- emits `WARNING: there is already a transaction in progress` and otherwise
-- leaves the transaction unchanged), so the later ROLLBACK unwinds from the
-- FIRST BEGIN and the connection-level catalog stays in sync with storage. If the
-- nested BEGIN reset the DDL-rollback baseline, the ROLLBACK would restore the
-- wrong catalog state and the re-CREATE below would error `relation already
-- exists` (catalog desynced from storage), failing setup.
-- Drives its own transactions, so it opts out of the harness's isolating wrapper
-- (and keeps its own DROP-first cleanup).
-- @setup
DROP TABLE IF EXISTS nested_begin_test CASCADE;
BEGIN;
CREATE TABLE nested_begin_test (id INT);
INSERT INTO nested_begin_test VALUES (1);
BEGIN;
ROLLBACK;
CREATE TABLE nested_begin_test (id INT, extra TEXT);
INSERT INTO nested_begin_test VALUES (2, 'ok');

-- @test
SELECT * FROM nested_begin_test ORDER BY id;
