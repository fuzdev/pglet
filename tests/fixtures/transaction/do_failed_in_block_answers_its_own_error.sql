-- @no-txn
-- Drives its own transaction, so it opts out of the harness's isolating wrapper (and
-- keeps its own DROP-first cleanup): the `DO` runs inside a block that has already
-- written. A body that creates a table, drops a committed one and writes before failing
-- answers the failing statement's `42P01` on both engines. The block is aborted after
-- it (`25P02` for what follows), which a fixture cannot express; that half, and what a
-- `ROLLBACK TO` over the failure leaves, is `crates/pglet/tests/do_block.rs`'s
-- `a_failed_do_inside_a_block_undoes_only_itself`.
-- @profile-lock
-- @setup
DROP TABLE IF EXISTS dof5_keep, dof5_gone, dof5_t CASCADE;
CREATE TABLE dof5_keep (id INT PRIMARY KEY);
INSERT INTO dof5_keep VALUES (1), (2);
CREATE TABLE dof5_gone (i INT);
INSERT INTO dof5_gone VALUES (7), (8);
BEGIN;
INSERT INTO dof5_keep VALUES (3);

-- @expect-error: the body's missing relation
-- @test
DO $$ BEGIN
    CREATE TABLE dof5_t (a INT);
    DROP TABLE dof5_gone;
    INSERT INTO dof5_keep VALUES (4);
    INSERT INTO dof5_nosuch VALUES (1);
END $$;
