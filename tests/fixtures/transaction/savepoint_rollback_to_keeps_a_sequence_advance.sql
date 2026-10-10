-- @no-txn
-- Drives its own transactions, so it opts out of the harness's isolating wrapper
-- (and keeps its own DROP-first cleanup).
--
-- **A sequence counter is not rolled back**, by a savepoint or by anything else:
-- `nextval` is non-transactional in Postgres, so the value drawn after the
-- savepoint stays drawn and the next one continues past it. Measured on 18.6 —
-- 1, savepoint, 2, `ROLLBACK TO`, then **3**.
--
-- It is a silent wrong in the other direction, which is why it is pinned: an
-- engine that restores the counter with the rest of the catalog hands out an id
-- it has already issued, and the collision surfaces as a `23505` in a statement
-- that did nothing wrong.
--
-- **The `CREATE TABLE` is load-bearing.** pglet skips the catalog restore
-- entirely for a region that wrote no catalog, and a `nextval` is not a catalog
-- write — so without any DDL in the savepoint's region the counter survives
-- because nothing was restored at all, and the fixture would score green against
-- an engine with no carry-forward rule whatsoever. Measured as a mutation:
-- deleting the carry-forward leaves this fixture green without the DDL and
-- reddens it with.
-- @setup
DROP SEQUENCE IF EXISTS sp_seq CASCADE;
DROP TABLE IF EXISTS sp_seq_ddl CASCADE;
CREATE SEQUENCE sp_seq;
BEGIN;
SELECT nextval('sp_seq');
SAVEPOINT s;
SELECT nextval('sp_seq');
CREATE TABLE sp_seq_ddl (a INT);
ROLLBACK TO s;

-- @test
SELECT nextval('sp_seq') AS after_rollback_to;
