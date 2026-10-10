-- @no-txn
-- A `DO` nested in a `DO` body whose own `EXCEPTION` condition does not match lets the
-- error through — the nearest this engine's minimal PL/pgSQL comes to an inner block
-- that re-raises, since a nested `BEGIN … EXCEPTION … END` and `RAISE` are not parsed.
-- The outer auto-commit statement answers the inner body's `42P01` on both engines; what
-- the two levels leave behind is read in `crates/pglet/tests/do_block.rs`'s
-- `a_failed_do_leaves_the_database_as_it_found_it`, the harness stopping at the error.
-- @setup
DROP TABLE IF EXISTS dof4_gone, dof4_t CASCADE;
DROP TYPE IF EXISTS dof4_e;
CREATE TABLE dof4_gone (i INT);
INSERT INTO dof4_gone VALUES (7), (8);

-- @expect-error: the inner body's missing relation
-- @test
DO $$ BEGIN
    CREATE TABLE dof4_t (a INT);
    DO $x$ BEGIN
        CREATE TYPE dof4_e AS ENUM ('a');
        DROP TABLE dof4_gone;
        INSERT INTO dof4_nosuch VALUES (1);
    EXCEPTION WHEN unique_violation THEN NULL;
    END $x$;
END $$;
