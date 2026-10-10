-- @no-txn
-- Drives its own transactions, so it opts out of the harness's isolating wrapper
-- (and keeps its own DROP-first cleanup).
--
-- The whole-block twin of `savepoint_rollback_to_keeps_a_sequence_advance.sql`,
-- and it needs the `CREATE TABLE` to reproduce: pglet skips the catalog restore
-- entirely for a block that touched no catalog, so a transaction of `nextval`
-- alone already answered correctly and hid the bug. With any DDL in the block the
-- restore ran and brought the **counter** back with it — `nextval` answered `1`
-- twice where 18.6 answers `1` then `2`, so the next insert collided on a key it
-- had already issued.
-- @setup
DROP SEQUENCE IF EXISTS rb_seq CASCADE;
DROP TABLE IF EXISTS rb_ddl CASCADE;
CREATE SEQUENCE rb_seq;
BEGIN;
SELECT nextval('rb_seq');
CREATE TABLE rb_ddl (a INT);
ROLLBACK;

-- @test
SELECT nextval('rb_seq') AS after_rollback;
