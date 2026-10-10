-- The `INSERT … SELECT` spelling of the precedence pair: the column list resolves before
-- the source's column count is compared with it, so a repeat outranks the mismatch.
-- pglet reported its own `INSERT has 2 target columns but the source query produces 1`.
--
-- Moving the resolution above that comparison also moved a name that is merely
-- **unknown** ahead of it — `INSERT INTO dtcsa (a, zz) SELECT 1` is `42703` on both
-- engines now, and was this same arity error here.
-- @setup
CREATE TABLE dtcsa (a INT, b INT);

-- @expect-error: column "a" specified more than once (42701)
-- @no-profile-lock: the column list is resolved while the row source is lowered, before any index selection runs
-- @test
INSERT INTO dtcsa (a, a) SELECT 1;
