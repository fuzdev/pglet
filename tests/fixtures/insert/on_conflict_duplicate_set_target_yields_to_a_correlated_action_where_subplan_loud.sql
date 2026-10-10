-- The same boundary reached through the action's own `WHERE`, whose sub-plan the
-- re-walk judges alongside the `RETURNING` one.
-- @setup
CREATE TABLE ocdscw (a INT UNIQUE, b INT);
CREATE TABLE ocdscwe (b INT);
INSERT INTO ocdscw VALUES (1, 1);

-- @expect-error: column "zzz" does not exist (42703)
-- @no-profile-lock: the refusal is raised while the statement is lowered, before the arbiter inference runs
-- @test
INSERT INTO ocdscw VALUES (1, 2) ON CONFLICT (a) DO UPDATE SET b = 1, b = 2 WHERE (SELECT b FROM ocdscwe ORDER BY zzz) = 1;
