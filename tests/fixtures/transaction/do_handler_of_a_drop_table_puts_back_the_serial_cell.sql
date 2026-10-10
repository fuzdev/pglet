-- @no-txn
-- Drives its own transactions, so it opts out of the harness's isolating wrapper
-- (and keeps its own DROP-first cleanup).
--
-- A caught `DO` body is rolled back before its handler runs, and that rollback puts
-- back the cell a body `DROP TABLE` replaced, as a block's does: the body re-creates the
-- table, draws `1`, then raises `23505` on its own row, and the handler swallows it.
-- 18.6 answers `4` after the `DO`, which runs in autocommit here — the executor's own
-- restore, with no block around it.
-- @setup
DROP TABLE IF EXISTS rdo_caught;
CREATE TABLE rdo_caught (id SERIAL PRIMARY KEY, v TEXT);
INSERT INTO rdo_caught (v) VALUES ('1'), ('2'), ('3');
DO $$ BEGIN DROP TABLE rdo_caught; CREATE TABLE rdo_caught (id SERIAL PRIMARY KEY, v TEXT); INSERT INTO rdo_caught (v) VALUES ('x'); INSERT INTO rdo_caught (id, v) VALUES (1, 'dup'); EXCEPTION WHEN OTHERS THEN NULL; END $$;
-- @test
INSERT INTO rdo_caught (v) VALUES ('n1') RETURNING id;
