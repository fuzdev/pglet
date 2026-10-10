-- @no-txn
-- Drives its own transactions, so it opts out of the harness's isolating wrapper
-- (and keeps its own DROP-first cleanup).
--
-- `DROP TABLE` takes the table's owned sequence with it, and a rollback puts the old
-- counter cell back. Here the block re-creates the table, whose new `SERIAL` cell draws
-- `1`, and the rollback must hand the old cell back at `3`, so the next inserts are `4`
-- and `5` — the rollback-per-test idiom over a test that re-creates its table. Measured
-- on 18.6. A restore that keeps the re-created cell's counter answers `23505` here.
-- @setup
DROP TABLE IF EXISTS rdt_pk;
CREATE TABLE rdt_pk (id SERIAL PRIMARY KEY, v TEXT);
INSERT INTO rdt_pk (v) VALUES ('1'), ('2'), ('3');
BEGIN;
DROP TABLE rdt_pk;
CREATE TABLE rdt_pk (id SERIAL PRIMARY KEY, v TEXT);
INSERT INTO rdt_pk (v) VALUES ('x');
ROLLBACK;
-- @test
INSERT INTO rdt_pk (v) VALUES ('n1'), ('n2') RETURNING id;
