-- The `RELEASE` member of `do_savepoint_in_body_is_0a000.sql`'s family, and not a
-- duplicate of it: the gate enumerates plan kinds, so each kind is a separate
-- claim, and 18.6 reaches this one through the same `0A000` while reaching
-- `ROLLBACK TO` through a `42601` instead (recorded as a divergence rather than
-- pinned).
-- @setup
CREATE TABLE do_tc2 (id INT);
-- @test
-- @expect-error: unsupported transaction command in PL/pgSQL (0A000)
DO $$ BEGIN
    INSERT INTO do_tc2 VALUES (1);
    RELEASE s;
END $$;
