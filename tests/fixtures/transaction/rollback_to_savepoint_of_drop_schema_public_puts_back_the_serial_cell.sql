-- @no-txn
-- Drives its own transactions, so it opts out of the harness's isolating wrapper
-- (and keeps its own DROP-first cleanup).
--
-- The reset idiom under a savepoint: `ROLLBACK TO` puts the table's old cell back at
-- `3`, the block draws `4`, and its rollback leaves that draw standing, so 18.6 answers
-- `5`. Keeping the re-created cell answers `23505`.
-- @setup
DROP TABLE IF EXISTS rds_sp;
CREATE TABLE rds_sp (id SERIAL PRIMARY KEY);
INSERT INTO rds_sp DEFAULT VALUES;
INSERT INTO rds_sp DEFAULT VALUES;
INSERT INTO rds_sp DEFAULT VALUES;
BEGIN;
SAVEPOINT a;
DROP SCHEMA public CASCADE;
CREATE SCHEMA public;
CREATE TABLE rds_sp (id SERIAL PRIMARY KEY);
INSERT INTO rds_sp DEFAULT VALUES;
ROLLBACK TO SAVEPOINT a;
INSERT INTO rds_sp DEFAULT VALUES RETURNING id;
ROLLBACK;
-- @test
INSERT INTO rds_sp DEFAULT VALUES RETURNING id;
