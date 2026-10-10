-- The guard's `WHERE` spelling of the same boundary.
-- @setup
CREATE TABLE igcw (id INT GENERATED ALWAYS AS IDENTITY, b INT);
CREATE TABLE igcwe (b INT);
INSERT INTO igcw (b) VALUES (0);

-- @expect-error: column "zzz" does not exist (42703)
-- @no-profile-lock: the refusal is raised while the statement is lowered, before any index selection runs
-- @test
UPDATE igcw SET id = 5 WHERE (SELECT b FROM igcwe ORDER BY zzz) = 1;
