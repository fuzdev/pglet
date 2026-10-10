-- @no-txn
-- Drives its own transactions, so it opts out of the harness's isolating wrapper
-- (and keeps its own DROP-first cleanup).
--
-- A `DO` that completes inside a block hands the counter cells its statements replaced
-- to the block — the host reads them off the `DO`'s result, since
-- `Plan::swapped_sequences` cannot list a `DO`'s ahead of it — so the block's rollback
-- puts each sequence back on its own cell as the in-place advances left it. Each
-- sequence is drawn to `3` first; the block draws `4` from `dcb_d` before its `DO`
-- replaces the cell. The routes: a body's `RESTART`, a handler's `RESTART` after its
-- body raised, a body's `DROP SEQUENCE` + `CREATE SEQUENCE`, a nested `DO`'s `RESTART`,
-- and a `DROP SEQUENCE` after the block's own draw. 18.6 answers `4 4 4 4 5`; handed
-- nothing, the rollback left `100 100 1 100 4` — the replacing cell's counter, or the
-- one at `BEGIN`.
-- @setup
DROP SEQUENCE IF EXISTS dcb_a, dcb_b, dcb_c, dcb_n, dcb_d;
CREATE SEQUENCE dcb_a;
CREATE SEQUENCE dcb_b;
CREATE SEQUENCE dcb_c;
CREATE SEQUENCE dcb_n;
CREATE SEQUENCE dcb_d;
SELECT nextval('dcb_a'), nextval('dcb_a'), nextval('dcb_a');
SELECT nextval('dcb_b'), nextval('dcb_b'), nextval('dcb_b');
SELECT nextval('dcb_c'), nextval('dcb_c'), nextval('dcb_c');
SELECT nextval('dcb_n'), nextval('dcb_n'), nextval('dcb_n');
SELECT nextval('dcb_d'), nextval('dcb_d'), nextval('dcb_d');
BEGIN;
SELECT nextval('dcb_d');
DO $$ BEGIN ALTER SEQUENCE dcb_a RESTART WITH 100; END $$;
DO $$ BEGIN INSERT INTO dcb_nosuch VALUES (1);
EXCEPTION WHEN OTHERS THEN ALTER SEQUENCE dcb_b RESTART WITH 100; END $$;
DO $$ BEGIN DROP SEQUENCE dcb_c; CREATE SEQUENCE dcb_c; END $$;
DO $o$ BEGIN DO $i$ BEGIN ALTER SEQUENCE dcb_n RESTART WITH 100; END $i$; END $o$;
DO $$ BEGIN DROP SEQUENCE dcb_d; END $$;
ROLLBACK;
-- @test
SELECT nextval('dcb_a') AS a, nextval('dcb_b') AS b, nextval('dcb_c') AS c,
       nextval('dcb_n') AS n, nextval('dcb_d') AS d;
