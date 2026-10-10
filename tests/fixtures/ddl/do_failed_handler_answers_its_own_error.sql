-- @no-txn
-- An auto-commit `DO` whose handler catches the body's error and then fails itself
-- answers the handler statement's error, `42P01`, on both engines — after the handler
-- has made a table, dropped a populated one and written a row, all of which the
-- statement's failure takes back with the body's (read in `crates/pglet/tests/do_block.rs`'s
-- `a_failed_do_leaves_the_database_as_it_found_it`, the harness stopping at the error).
-- @profile-lock
-- @setup
DROP TABLE IF EXISTS dof3_keep, dof3_gone, dof3_t, dof3_h CASCADE;
CREATE TABLE dof3_keep (id INT PRIMARY KEY);
INSERT INTO dof3_keep VALUES (1), (2);
CREATE TABLE dof3_gone (i INT);
INSERT INTO dof3_gone VALUES (7), (8);

-- @expect-error: the handler's missing relation
-- @test
DO $$ BEGIN
    CREATE TABLE dof3_t (a INT);
    INSERT INTO dof3_nosuch VALUES (1);
EXCEPTION WHEN others THEN
    CREATE TABLE dof3_h (a INT);
    DROP TABLE dof3_gone;
    INSERT INTO dof3_keep VALUES (3);
    INSERT INTO dof3_nosuch_either VALUES (2);
END $$;
