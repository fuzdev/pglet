-- @no-txn
-- Drives its own transactions, so it opts out of the harness's isolating wrapper.
--
-- A repeated savepoint name **stacks**; it does not collide and does not replace.
-- `ROLLBACK TO s` takes the most recent `s` (so `3` is gone and `2` survives, and
-- repeating it changes nothing), and once `RELEASE s` has dropped that one the
-- *older* `s` answers the next reference — leaving only `1`. Measured on 18.6;
-- an implementation keyed on a name→frame map answers the last `ROLLBACK TO`
-- with `3B001` instead.
-- @setup
DROP TABLE IF EXISTS sp_dup CASCADE;
CREATE TABLE sp_dup (id INT);
BEGIN;
INSERT INTO sp_dup VALUES (1);
SAVEPOINT s;
INSERT INTO sp_dup VALUES (2);
SAVEPOINT s;
INSERT INTO sp_dup VALUES (3);
ROLLBACK TO s;
ROLLBACK TO s;
RELEASE s;
ROLLBACK TO s;
COMMIT;

-- @test
SELECT id FROM sp_dup ORDER BY id;
