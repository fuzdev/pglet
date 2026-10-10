-- @no-txn
-- An auto-commit `DO` whose body creates a table, drops a populated one and writes a row
-- before a unique violation answers that violation's `23505` on both engines: the
-- block's undo runs before the error returns and does not replace it. The state the
-- failure leaves is read in `crates/pglet/tests/do_block.rs`'s
-- `a_failed_do_leaves_the_database_as_it_found_it`, the harness stopping at the error.
-- @profile-lock
-- @setup
DROP TABLE IF EXISTS dof2_keep, dof2_gone, dof2_t CASCADE;
CREATE TABLE dof2_keep (id INT PRIMARY KEY, v TEXT);
INSERT INTO dof2_keep VALUES (1, 'a'), (2, 'b');
CREATE TABLE dof2_gone (i INT);
INSERT INTO dof2_gone VALUES (7), (8);

-- @expect-error: the body's duplicate key
-- @test
DO $$ BEGIN
    CREATE TABLE dof2_t (a INT);
    DROP TABLE dof2_gone;
    INSERT INTO dof2_keep VALUES (3, 'c');
    INSERT INTO dof2_keep VALUES (1, 'dup');
END $$;
