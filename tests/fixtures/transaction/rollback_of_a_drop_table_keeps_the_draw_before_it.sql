-- @no-txn
-- Drives its own transactions, so it opts out of the harness's isolating wrapper
-- (and keeps its own DROP-first cleanup).
--
-- The value put back is the one the old cell held **just before the drop**, not the
-- one it held at `BEGIN`: the block's insert draws `4` in place, and a `nextval` is not
-- rolled back, so the next insert after the rollback is `5` on 18.6. Restoring the
-- `BEGIN` value re-issues `4`.
-- @setup
DROP TABLE IF EXISTS rdt_draw;
CREATE TABLE rdt_draw (id SERIAL PRIMARY KEY, v TEXT);
INSERT INTO rdt_draw (v) VALUES ('1'), ('2'), ('3');
BEGIN;
INSERT INTO rdt_draw (v) VALUES ('x') RETURNING id;
DROP TABLE rdt_draw;
ROLLBACK;
-- @test
INSERT INTO rdt_draw (v) VALUES ('n1') RETURNING id;
