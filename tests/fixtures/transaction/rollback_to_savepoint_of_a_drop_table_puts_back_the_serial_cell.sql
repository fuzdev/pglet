-- @no-txn
-- Drives its own transactions, so it opts out of the harness's isolating wrapper
-- (and keeps its own DROP-first cleanup).
--
-- The savepoint form, which a `transaction()` helper under a per-test block produces: the
-- drop and re-create sit under a savepoint that is rolled back to and released, the
-- block draws `4` from the restored old cell, and the block's own rollback leaves that
-- draw standing, so 18.6 answers `5`.
-- @setup
DROP TABLE IF EXISTS rsp_pk;
CREATE TABLE rsp_pk (id SERIAL PRIMARY KEY, v TEXT);
INSERT INTO rsp_pk (v) VALUES ('1'), ('2'), ('3');
BEGIN;
SAVEPOINT tx1;
DROP TABLE rsp_pk;
CREATE TABLE rsp_pk (id SERIAL PRIMARY KEY, v TEXT);
INSERT INTO rsp_pk (v) VALUES ('x');
ROLLBACK TO SAVEPOINT tx1;
RELEASE SAVEPOINT tx1;
INSERT INTO rsp_pk (v) VALUES ('in-block') RETURNING id;
ROLLBACK;
-- @test
INSERT INTO rsp_pk (v) VALUES ('n1') RETURNING id;
