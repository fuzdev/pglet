-- A `SET` target the relation lacks, written twice, is the unknown name: the duplicate
-- gate is keyed on a column the statement resolved, and this one never resolves.
-- @setup
CREATE TABLE dstz (a INT, b INT);
INSERT INTO dstz VALUES (0, 0);

-- @expect-error: column "zz" of relation "dstz" does not exist (42703)
-- @no-profile-lock: the refusal is raised while the statement is lowered, before any index selection runs
-- @test
UPDATE dstz SET zz = 1, zz = 2;
