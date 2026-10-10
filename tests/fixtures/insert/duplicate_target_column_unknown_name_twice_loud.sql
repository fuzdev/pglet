-- A name the relation lacks, written twice, is the unknown name and not the duplicate:
-- the resolution is what the claim is keyed on, so a name that never resolves never
-- claims anything.
-- @setup
CREATE TABLE dtcz (a INT, b INT);

-- @expect-error: column "zz" of relation "dtcz" does not exist (42703)
-- @no-profile-lock: the column list is resolved while the row source is lowered, before any index selection runs
-- @test
INSERT INTO dtcz (zz, zz) VALUES (1, 2);
