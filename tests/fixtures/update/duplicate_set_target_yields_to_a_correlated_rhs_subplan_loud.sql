-- And through a `SET` right-hand side, the third clause that can carry a sub-plan into
-- the re-walk. The repeated target is the *second* assignment, so nothing but the gate's
-- position decides this one.
-- @setup
CREATE TABLE dscs (a INT, b INT);
CREATE TABLE dscse (b INT);
INSERT INTO dscs VALUES (0, 0);

-- @expect-error: column "zzz" does not exist (42703)
-- @no-profile-lock: the refusal is raised while the statement is lowered, before any index selection runs
-- @test
UPDATE dscs SET a = (SELECT b FROM dscse ORDER BY zzz), a = 2;
