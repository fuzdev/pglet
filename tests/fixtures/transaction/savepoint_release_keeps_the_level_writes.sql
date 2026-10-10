-- @no-txn
-- Drives its own transactions, so it opts out of the harness's isolating wrapper.
--
-- `RELEASE` is the other half of `ROLLBACK TO` and the polarity has to be pinned
-- separately: the level's writes **merge into the enclosing one** rather than
-- being discarded, and the enclosing block's `COMMIT` publishes them. An
-- implementation that restored on release would leave only `1` here.
-- @setup
DROP TABLE IF EXISTS sp_release CASCADE;
CREATE TABLE sp_release (id INT);
BEGIN;
INSERT INTO sp_release VALUES (1);
SAVEPOINT s;
INSERT INTO sp_release VALUES (2);
RELEASE s;
COMMIT;

-- @test
SELECT id FROM sp_release ORDER BY id;
