-- @no-txn
-- Drives its own transactions, so it opts out of the harness's isolating wrapper.
--
-- **`ROLLBACK TO` does not destroy the savepoint it names**, so the same one
-- answers twice and a `RELEASE` of it afterwards still succeeds. This is the
-- discriminating fixture for the whole design: an implementation that *pops* the
-- frame to restore it passes every other savepoint fixture here and answers the
-- second `ROLLBACK TO` with `3B001` — and the `RELEASE` too. Measured on 18.6.
-- @setup
DROP TABLE IF EXISTS sp_repeat CASCADE;
CREATE TABLE sp_repeat (id INT);
BEGIN;
INSERT INTO sp_repeat VALUES (1);
SAVEPOINT s;
INSERT INTO sp_repeat VALUES (2);
ROLLBACK TO s;
INSERT INTO sp_repeat VALUES (3);
ROLLBACK TO s;
RELEASE s;
COMMIT;

-- @test
SELECT id FROM sp_repeat ORDER BY id;
