-- @no-txn
-- Drives its own transactions, so it opts out of the harness's isolating wrapper
-- (and keeps its own DROP-first cleanup).
--
-- A sequence `OWNED BY` a column that draws nothing from it is still dropped with the
-- table (the owner edge decides, not a default), and the rollback puts its old cell
-- back at `3`: the block re-creates the sequence and draws `1`–`5` from the new cell,
-- and 18.6 answers `4`.
-- @setup
DROP TABLE IF EXISTS rdt_owned;
CREATE TABLE rdt_owned (x INT);
CREATE SEQUENCE rdt_owned_s OWNED BY rdt_owned.x;
SELECT nextval('rdt_owned_s');
SELECT nextval('rdt_owned_s');
SELECT nextval('rdt_owned_s');
BEGIN;
DROP TABLE rdt_owned;
CREATE SEQUENCE rdt_owned_s;
SELECT nextval('rdt_owned_s');
SELECT nextval('rdt_owned_s');
SELECT nextval('rdt_owned_s');
SELECT nextval('rdt_owned_s');
SELECT nextval('rdt_owned_s');
ROLLBACK;
-- @test
SELECT nextval('rdt_owned_s');
