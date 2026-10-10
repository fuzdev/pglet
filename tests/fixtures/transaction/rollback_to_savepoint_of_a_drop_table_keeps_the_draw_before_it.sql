-- @no-txn
-- Drives its own transactions, so it opts out of the harness's isolating wrapper
-- (and keeps its own DROP-first cleanup).
--
-- A draw then a drop under a savepoint: `ROLLBACK TO` puts the old cell back at `4`,
-- the draw it holds, so the block's next insert is `5`; the block's rollback keeps that
-- too, and 18.6 answers `6`.
-- @setup
DROP TABLE IF EXISTS rsp_draw;
CREATE TABLE rsp_draw (id SERIAL PRIMARY KEY);
INSERT INTO rsp_draw DEFAULT VALUES;
INSERT INTO rsp_draw DEFAULT VALUES;
INSERT INTO rsp_draw DEFAULT VALUES;
BEGIN;
SAVEPOINT a;
INSERT INTO rsp_draw DEFAULT VALUES RETURNING id;
DROP TABLE rsp_draw;
ROLLBACK TO SAVEPOINT a;
INSERT INTO rsp_draw DEFAULT VALUES RETURNING id;
ROLLBACK;
-- @test
INSERT INTO rsp_draw DEFAULT VALUES RETURNING id;
