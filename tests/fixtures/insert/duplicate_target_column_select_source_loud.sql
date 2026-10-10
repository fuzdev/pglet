-- The `INSERT … SELECT` source maps its columns through a different function from the
-- `VALUES` one (`insert_select_target_indices` against the `VALUES` arm's mapping), so
-- the rule has to be read off the column list both share. Without it this wrote too.
-- @setup
CREATE TABLE dtcs (a INT, b INT);

-- @expect-error: column "a" specified more than once (42701)
-- @no-profile-lock: the column list is resolved while the row source is lowered, before any index selection runs
-- @test
INSERT INTO dtcs (a, a) SELECT 1, 2;
