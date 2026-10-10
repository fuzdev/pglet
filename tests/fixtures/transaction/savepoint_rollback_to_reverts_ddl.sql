-- @no-txn
-- Drives its own transactions, so it opts out of the harness's isolating wrapper
-- (and keeps its own DROP-first cleanup).
--
-- A savepoint rewinds the **catalog** as well as the rows: a table created after
-- it is gone, and a row inserted after it into a table that predates it is gone
-- too. Both in one fixture because they take different channels here — the
-- catalog through the session's per-frame `Schema` snapshot, the row through the
-- backend's pending buffer — and a fixture that saw only one would score green
-- with the other unimplemented.
-- @setup
DROP TABLE IF EXISTS sp_ddl CASCADE;
DROP TABLE IF EXISTS sp_ddl2 CASCADE;
BEGIN;
CREATE TABLE sp_ddl (a INT);
SAVEPOINT s;
CREATE TABLE sp_ddl2 (a INT);
INSERT INTO sp_ddl VALUES (7);
ROLLBACK TO s;
COMMIT;

-- @test
SELECT
  (SELECT count(*) FROM sp_ddl) AS rows_in_sp_ddl,
  (SELECT count(*) FROM information_schema.tables WHERE table_name = 'sp_ddl2') AS sp_ddl2_exists;
