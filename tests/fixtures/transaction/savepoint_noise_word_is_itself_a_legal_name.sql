-- @no-txn
-- Drives its own transactions, so it opts out of the harness's isolating wrapper
-- (and keeps its own DROP-first cleanup).
--
-- `SAVEPOINT` is an unreserved keyword, so it is a legal savepoint name — and
-- `gram.y` separates `RELEASE SAVEPOINT ColId` from `RELEASE ColId` with a whole
-- extra production to keep both spellings. `RELEASE savepoint` must therefore
-- release the savepoint *named* `savepoint`, not be read as a `RELEASE` missing
-- its argument; `ROLLBACK TO savepoint` likewise. Both spellings appear here, and
-- the `2` proves the release merged rather than restored.
-- @setup
DROP TABLE IF EXISTS sp_noise CASCADE;
CREATE TABLE sp_noise (id INT);
BEGIN;
SAVEPOINT savepoint;
INSERT INTO sp_noise VALUES (1);
ROLLBACK TO savepoint;
INSERT INTO sp_noise VALUES (2);
RELEASE savepoint;
COMMIT;

-- @test
SELECT id FROM sp_noise ORDER BY id;
