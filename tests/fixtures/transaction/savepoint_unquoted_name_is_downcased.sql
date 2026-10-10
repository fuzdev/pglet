-- @no-txn
-- Drives its own transactions, so it opts out of the harness's isolating wrapper
-- (and keeps its own DROP-first cleanup).
--
-- A savepoint name is a `ColId`, so the scanner downcases an unquoted spelling:
-- `SAVEPOINT MySp` establishes `mysp`, and `ROLLBACK TO mysp` finds it. The proof
-- is the **rows**, not an error — under a parser that kept the name verbatim the
-- `ROLLBACK TO` raises `3B001` in setup, and the fixture reports no rows where
-- 18.6 reports one.
--
-- It is deliberately not an `@expect-error` fixture, and the reason generalizes:
-- the harness reports a `@setup` failure as the fixture's whole result, so an
-- error fixture whose setup can fail with the **same** SQLSTATE it is asserting
-- scores green off the wrong statement. That is exactly what the first version of
-- this fixture did — measured as a mutation, it stayed green with the fold
-- removed.
-- @setup
DROP TABLE IF EXISTS sp_fold CASCADE;
CREATE TABLE sp_fold (id INT);
BEGIN;
INSERT INTO sp_fold VALUES (1);
SAVEPOINT MySp;
INSERT INTO sp_fold VALUES (2);
ROLLBACK TO mysp;
COMMIT;

-- @test
SELECT id FROM sp_fold ORDER BY id;
