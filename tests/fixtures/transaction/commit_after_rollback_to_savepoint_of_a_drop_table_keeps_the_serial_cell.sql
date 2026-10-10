-- @no-txn
-- Drives its own transactions, so it opts out of the harness's isolating wrapper
-- (and keeps its own DROP-first cleanup).
--
-- `ROLLBACK TO` a savepoint under which the table was dropped and re-created, then
-- `COMMIT`: the committed catalog holds the old cell at `3`, and 18.6 answers `4`.
-- @setup
DROP TABLE IF EXISTS rsp_commit;
CREATE TABLE rsp_commit (id SERIAL PRIMARY KEY);
INSERT INTO rsp_commit DEFAULT VALUES;
INSERT INTO rsp_commit DEFAULT VALUES;
INSERT INTO rsp_commit DEFAULT VALUES;
BEGIN;
SAVEPOINT s;
DROP TABLE rsp_commit;
CREATE TABLE rsp_commit (id SERIAL PRIMARY KEY);
INSERT INTO rsp_commit DEFAULT VALUES;
ROLLBACK TO SAVEPOINT s;
COMMIT;
-- @test
INSERT INTO rsp_commit DEFAULT VALUES RETURNING id;
