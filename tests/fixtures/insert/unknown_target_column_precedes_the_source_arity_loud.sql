-- The `INSERT … SELECT` column list resolving ahead of the source-arity comparison
-- carried a merely **unknown** name with it: this was pglet's `INSERT has 2 target
-- columns but the source query produces 1` and is `42703` on both engines now, where
-- `checkInsertTargets` raises it. Its duplicate twin is
-- `duplicate_target_column_precedes_the_source_arity_loud.sql`.
-- @setup
CREATE TABLE utsa (a INT, b INT);

-- @expect-error: column "zz" of relation "utsa" does not exist (42703)
-- @no-profile-lock: the column list is resolved while the row source is lowered, before any index selection runs
-- @test
INSERT INTO utsa (a, zz) SELECT 1;
