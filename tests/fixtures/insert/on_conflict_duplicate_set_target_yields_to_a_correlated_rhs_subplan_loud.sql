-- And through an action right-hand side, where the repeated target is the second
-- assignment and only the gate's position decides the verdict.
-- @setup
CREATE TABLE ocdscs (a INT UNIQUE, b INT);
CREATE TABLE ocdscse (b INT);
INSERT INTO ocdscs VALUES (1, 1);

-- @expect-error: column "zzz" does not exist (42703)
-- @no-profile-lock: the refusal is raised while the statement is lowered, before the arbiter inference runs
-- @test
INSERT INTO ocdscs VALUES (1, 2) ON CONFLICT (a) DO UPDATE SET b = (SELECT b FROM ocdscse ORDER BY zzz), b = 3;
