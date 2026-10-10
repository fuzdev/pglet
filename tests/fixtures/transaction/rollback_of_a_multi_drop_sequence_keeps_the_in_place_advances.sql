-- @no-txn
-- Drives its own transactions, so it opts out of the harness's isolating wrapper
-- (and keeps its own DROP-first cleanup).
--
-- `DROP SEQUENCE` gives each named sequence a **new** counter cell, so a rollback
-- puts the old cell back — as the in-place, non-transactional `nextval`s left it, not
-- as it stood at `BEGIN`. The engine therefore has to read the outgoing value of
-- **every** name before the statement runs, and the observable is deliberately the
-- **second** one: an arm that records only the first name of the list restores `dsw2`
-- from the snapshot instead and answers `1`. Measured on 18.6: `3`.
--
-- `sequences/drop_sequence.sql` and the single-name rollback fixtures cannot see this
-- — with one name, "the list" and "its first element" coincide.
-- @setup
DROP SEQUENCE IF EXISTS dsw1 CASCADE;
DROP SEQUENCE IF EXISTS dsw2 CASCADE;
CREATE SEQUENCE dsw1;
CREATE SEQUENCE dsw2;
BEGIN;
SELECT nextval('dsw2');
SELECT nextval('dsw2');
DROP SEQUENCE dsw1, dsw2;
ROLLBACK;

-- @test
SELECT nextval('dsw2') AS after_rollback;
