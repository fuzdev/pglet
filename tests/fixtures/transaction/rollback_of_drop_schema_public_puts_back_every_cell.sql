-- @no-txn
-- Drives its own transactions, so it opts out of the harness's isolating wrapper
-- (and keeps its own DROP-first cleanup).
--
-- `DROP SCHEMA public CASCADE` — the migration and test reset idiom — clears every
-- sequence, owned or not, and a rollback puts each old cell back as the block's
-- in-place draws left it: the standalone sequence at `4` (one draw in the block before
-- the drop) and the table's at `1`, so 18.6 answers `5` and `2`. Keeping the re-created
-- cells answers `7` and `4`.
-- @setup
DROP TABLE IF EXISTS rds_t;
DROP SEQUENCE IF EXISTS rds_s;
CREATE SEQUENCE rds_s;
SELECT nextval('rds_s');
SELECT nextval('rds_s');
SELECT nextval('rds_s');
CREATE TABLE rds_t (id SERIAL PRIMARY KEY);
INSERT INTO rds_t DEFAULT VALUES;
BEGIN;
SELECT nextval('rds_s');
DROP SCHEMA public CASCADE;
CREATE SCHEMA public;
CREATE SEQUENCE rds_s;
SELECT nextval('rds_s');
SELECT nextval('rds_s');
SELECT nextval('rds_s');
SELECT nextval('rds_s');
SELECT nextval('rds_s');
SELECT nextval('rds_s');
CREATE TABLE rds_t (id SERIAL PRIMARY KEY);
INSERT INTO rds_t DEFAULT VALUES;
INSERT INTO rds_t DEFAULT VALUES;
INSERT INTO rds_t DEFAULT VALUES;
ROLLBACK;
-- @test
SELECT nextval('rds_s') AS s, nextval('rds_t_id_seq') AS t;
