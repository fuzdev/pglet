-- @no-txn
-- Drives its own transactions, so it opts out of the harness's isolating wrapper
-- (and keeps its own DROP-first cleanup).
--
-- A `DROP TABLE` naming two tables takes each one's owned sequence, and the rollback
-- puts back each old cell: `4` and `3` on 18.6, one per table.
-- @setup
DROP TABLE IF EXISTS rdt_multi_a, rdt_multi_b;
CREATE TABLE rdt_multi_a (id SERIAL PRIMARY KEY);
CREATE TABLE rdt_multi_b (id SERIAL PRIMARY KEY);
INSERT INTO rdt_multi_a DEFAULT VALUES;
INSERT INTO rdt_multi_a DEFAULT VALUES;
INSERT INTO rdt_multi_a DEFAULT VALUES;
INSERT INTO rdt_multi_b DEFAULT VALUES;
INSERT INTO rdt_multi_b DEFAULT VALUES;
BEGIN;
DROP TABLE rdt_multi_a, rdt_multi_b;
CREATE TABLE rdt_multi_a (id SERIAL PRIMARY KEY);
CREATE TABLE rdt_multi_b (id SERIAL PRIMARY KEY);
INSERT INTO rdt_multi_a DEFAULT VALUES;
INSERT INTO rdt_multi_b DEFAULT VALUES;
ROLLBACK;
-- @test
SELECT nextval('rdt_multi_a_id_seq') AS a, nextval('rdt_multi_b_id_seq') AS b;
