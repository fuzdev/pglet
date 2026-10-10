-- Half of the precedence pair that says **where** the gate runs. `checkInsertTargets`
-- resolves the column list before `transformInsertRow` counts the tuple, so a list with a
-- repeat is `42701` even when the tuple is the wrong length — where pglet reported its
-- own arity error (`INSERT has 1 expressions but 2 target columns`, `22000`).
--
-- The other half is `duplicate_target_column_precedes_the_cell_cast_loud.sql`, and the
-- `INSERT … SELECT` spelling is
-- `duplicate_target_column_precedes_the_source_arity_loud.sql`.
-- @setup
CREATE TABLE dtcap (a INT, b INT);

-- @expect-error: column "a" specified more than once (42701)
-- @no-profile-lock: the column list is resolved while the row source is lowered, before any index selection runs
-- @test
INSERT INTO dtcap (a, a) VALUES (1);
