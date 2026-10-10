-- @no-txn
-- Drives its own transactions, so it opts out of the harness's isolating wrapper
-- (and keeps its own DROP-first cleanup).
--
-- The re-created cell can land **above** the old one, and the rollback still puts the
-- old one back: ten draws in the block leave the new cell at `10`, and 18.6 answers `4`
-- after the rollback. Keeping the new cell's counter answers `11` — no collision, a
-- silent skip.
-- @setup
DROP TABLE IF EXISTS rdt_above;
CREATE TABLE rdt_above (id SERIAL PRIMARY KEY, v TEXT);
INSERT INTO rdt_above (v) VALUES ('1'), ('2'), ('3');
BEGIN;
DROP TABLE rdt_above;
CREATE TABLE rdt_above (id SERIAL PRIMARY KEY, v TEXT);
INSERT INTO rdt_above (v) VALUES ('x'), ('x'), ('x'), ('x'), ('x'), ('x'), ('x'), ('x'), ('x'), ('x');
ROLLBACK;
-- @test
INSERT INTO rdt_above (v) VALUES ('n1') RETURNING id;
