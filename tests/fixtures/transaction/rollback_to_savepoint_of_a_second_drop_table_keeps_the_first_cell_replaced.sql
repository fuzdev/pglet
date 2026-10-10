-- @no-txn
-- Drives its own transactions, so it opts out of the harness's isolating wrapper
-- (and keeps its own DROP-first cleanup).
--
-- The block replaces the cell before the savepoint and again under it. `ROLLBACK TO`
-- takes back only the second replacement: the block's re-created cell comes back at
-- `2`, its own draws, and the block draws `3`. The block's rollback then puts back the
-- cell it started with, at `3`, so 18.6 answers `4`.
-- @setup
DROP TABLE IF EXISTS rsp_nested;
CREATE TABLE rsp_nested (id SERIAL PRIMARY KEY);
INSERT INTO rsp_nested DEFAULT VALUES;
INSERT INTO rsp_nested DEFAULT VALUES;
INSERT INTO rsp_nested DEFAULT VALUES;
BEGIN;
DROP TABLE rsp_nested;
CREATE TABLE rsp_nested (id SERIAL PRIMARY KEY);
INSERT INTO rsp_nested DEFAULT VALUES;
INSERT INTO rsp_nested DEFAULT VALUES;
SAVEPOINT a;
DROP TABLE rsp_nested;
CREATE TABLE rsp_nested (id SERIAL PRIMARY KEY);
ROLLBACK TO SAVEPOINT a;
INSERT INTO rsp_nested DEFAULT VALUES RETURNING id;
ROLLBACK;
-- @test
INSERT INTO rsp_nested DEFAULT VALUES RETURNING id;
