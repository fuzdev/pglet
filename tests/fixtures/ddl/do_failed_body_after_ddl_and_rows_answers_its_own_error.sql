-- @no-txn
-- Runs outside the harness's isolating wrapper so the `DO` is an auto-commit statement,
-- the route whose failure the block's own undo covers (it keeps its own DROP-first
-- cleanup). A body that makes a sequence, a table with its serial, unique and check, a
-- type, an index and a column, writes rows, drops a populated table and then fails answers
-- the failing statement's own `42P01` on both engines — the undo that runs before the
-- error returns does not replace it.
--
-- What the failure leaves behind cannot be read here: the harness stops at the first
-- error, so the state half is `crates/pglet/tests/do_block.rs`'s
-- `a_failed_do_leaves_the_database_as_it_found_it`, over a body of the same shape.
-- @profile-lock
-- @setup
DROP TABLE IF EXISTS dof_keep, dof_gone, dof_t CASCADE;
DROP SEQUENCE IF EXISTS dof_s;
DROP TYPE IF EXISTS dof_e;
CREATE TABLE dof_keep (id INT PRIMARY KEY, v TEXT);
INSERT INTO dof_keep VALUES (1, 'a'), (2, 'b');
CREATE TABLE dof_gone (i INT);
INSERT INTO dof_gone VALUES (7), (8);

-- @expect-error: the body's missing relation
-- @test
DO $$ BEGIN
    CREATE SEQUENCE dof_s;
    CREATE TABLE dof_t (id SERIAL PRIMARY KEY, u INT UNIQUE, c INT CHECK (c > 0));
    INSERT INTO dof_t (u, c) VALUES (1, 1);
    CREATE TYPE dof_e AS ENUM ('a');
    CREATE INDEX dof_keep_v ON dof_keep (v);
    ALTER TABLE dof_keep ADD COLUMN w INT DEFAULT 5;
    INSERT INTO dof_keep (id, v) VALUES (3, 'c');
    DROP TABLE dof_gone;
    INSERT INTO dof_nosuch VALUES (1);
END $$;
