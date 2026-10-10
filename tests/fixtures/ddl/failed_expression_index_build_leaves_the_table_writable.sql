-- @no-txn
-- The route that needs no refused type and no over-rejection: **both** engines raise
-- `22012` building this index, over an ordinary `INT` column. So the `CREATE INDEX` is
-- not a divergence at all, and everything the fixture compares afterwards is pglet's
-- alone — a failed build that left its storage def behind made the *next* write of a
-- zero raise against an index the statement never created.
--
-- **This no longer reaches the storage build.** The executor sizes every entry a build
-- would write before it asks storage for the build, and sizing evaluates the key
-- expression — so the `22012` is raised there, ahead of the step whose residue this
-- fixture was written for. Measured: with the storage def registered ahead of the
-- back-fill again, this fixture stays green and
-- `failed_gin_index_build_leaves_the_table_writable.sql` reddens. What it pins
-- is the statement-level property — a `CREATE INDEX` both engines refuse leaves the
-- table writable — and the two engines taking the identical path through the `DO`
-- handler. That sibling states the harness limits this one shares.
-- @setup
DROP TABLE IF EXISTS ebw CASCADE;
CREATE TABLE ebw (a INT);
INSERT INTO ebw VALUES (0), (1);
DO $$ BEGIN
    CREATE INDEX ebw_ix ON ebw ((10 / a));
EXCEPTION WHEN others THEN NULL;
END $$;
INSERT INTO ebw VALUES (0);

-- @test
SELECT count(*) AS n FROM ebw;
