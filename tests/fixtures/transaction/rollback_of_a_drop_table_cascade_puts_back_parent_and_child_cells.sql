-- @no-txn
-- Drives its own transactions, so it opts out of the harness's isolating wrapper
-- (and keeps its own DROP-first cleanup).
--
-- `DROP TABLE … CASCADE` over a table a foreign key references sheds the child's
-- constraint; the block then drops the child and re-creates both. The rollback puts
-- back both old cells, so 18.6 answers `4` for each.
-- @setup
DROP TABLE IF EXISTS rdt_casc_c, rdt_casc_p;
CREATE TABLE rdt_casc_p (id SERIAL PRIMARY KEY);
CREATE TABLE rdt_casc_c (id SERIAL PRIMARY KEY, pid INT REFERENCES rdt_casc_p (id));
INSERT INTO rdt_casc_p DEFAULT VALUES;
INSERT INTO rdt_casc_p DEFAULT VALUES;
INSERT INTO rdt_casc_p DEFAULT VALUES;
INSERT INTO rdt_casc_c (pid) VALUES (1), (2), (3);
BEGIN;
DROP TABLE rdt_casc_p CASCADE;
DROP TABLE rdt_casc_c;
CREATE TABLE rdt_casc_p (id SERIAL PRIMARY KEY);
CREATE TABLE rdt_casc_c (id SERIAL PRIMARY KEY, pid INT REFERENCES rdt_casc_p (id));
INSERT INTO rdt_casc_p DEFAULT VALUES;
ROLLBACK;
INSERT INTO rdt_casc_p DEFAULT VALUES;
-- @test
INSERT INTO rdt_casc_c (pid) VALUES (1) RETURNING id, (SELECT max(id) FROM rdt_casc_p) AS parent_id;
