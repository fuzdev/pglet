-- @no-txn
-- Drives its own transactions, so it opts out of the harness's isolating wrapper
-- (and keeps its own DROP-first cleanup).
--
-- A savepoint that is **released** merges its drop into the block, and the block's
-- rollback puts the old cell back: 18.6 answers `4`.
-- @setup
DROP TABLE IF EXISTS rsp_released;
CREATE TABLE rsp_released (id SERIAL PRIMARY KEY, v TEXT);
INSERT INTO rsp_released (v) VALUES ('1'), ('2'), ('3');
BEGIN;
SAVEPOINT s;
DROP TABLE rsp_released;
CREATE TABLE rsp_released (id SERIAL PRIMARY KEY, v TEXT);
INSERT INTO rsp_released (v) VALUES ('y');
RELEASE SAVEPOINT s;
ROLLBACK;
-- @test
INSERT INTO rsp_released (v) VALUES ('n1') RETURNING id;
