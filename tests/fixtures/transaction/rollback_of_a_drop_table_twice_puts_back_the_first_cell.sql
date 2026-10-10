-- @no-txn
-- Drives its own transactions, so it opts out of the harness's isolating wrapper
-- (and keeps its own DROP-first cleanup).
--
-- Two replacements of one name in a block: the second `DROP TABLE` drops the cell the
-- first one's re-create made, and the rollback must put back the cell the block
-- started with, at `3`, so 18.6 answers `4`. Reading the second drop's outgoing value
-- instead (`8`) answers `9`.
-- @setup
DROP TABLE IF EXISTS rdt_twice;
CREATE TABLE rdt_twice (id SERIAL PRIMARY KEY);
INSERT INTO rdt_twice DEFAULT VALUES;
INSERT INTO rdt_twice DEFAULT VALUES;
INSERT INTO rdt_twice DEFAULT VALUES;
BEGIN;
DROP TABLE rdt_twice;
CREATE TABLE rdt_twice (id SERIAL PRIMARY KEY);
INSERT INTO rdt_twice DEFAULT VALUES;
INSERT INTO rdt_twice DEFAULT VALUES;
INSERT INTO rdt_twice DEFAULT VALUES;
INSERT INTO rdt_twice DEFAULT VALUES;
INSERT INTO rdt_twice DEFAULT VALUES;
INSERT INTO rdt_twice DEFAULT VALUES;
INSERT INTO rdt_twice DEFAULT VALUES;
INSERT INTO rdt_twice DEFAULT VALUES;
DROP TABLE rdt_twice;
CREATE TABLE rdt_twice (id SERIAL PRIMARY KEY);
ROLLBACK;
-- @test
INSERT INTO rdt_twice DEFAULT VALUES RETURNING id;
