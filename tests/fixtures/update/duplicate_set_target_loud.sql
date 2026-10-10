-- An `UPDATE`'s `SET` list is a **set**: a column assigned twice is
-- `42601 multiple assignments to same column "a"`, from `process_matched_tle`
-- (`rewrite/rewriteHandler.c`), which merges the repeated entry where it can and refuses
-- it where it cannot. pglet has no assignment it could merge, so every repeat is the
-- refusal. It accepted the list and wrote the **last** value, leaving `a = 2`.
--
-- The SQLSTATE differs from the `INSERT` column list's `42701` because the two are
-- raised from different phases, not because the invariant differs; the `INSERT` half is
-- `insert/duplicate_target_column_loud.sql`.
-- @setup
CREATE TABLE dst (a INT, b INT, c INT);
INSERT INTO dst VALUES (0, 0, 0);

-- @expect-error: multiple assignments to same column "a" (42601)
-- @no-profile-lock: the refusal is raised while the statement is lowered, before any index selection runs
-- @test
UPDATE dst SET a = 1, a = 2;
