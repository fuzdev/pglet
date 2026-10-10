-- The `ON CONFLICT DO UPDATE SET` twin of the `UPDATE` gate's last boundary. This one
-- was already below the correlated re-walk when it landed; the fixture is what pins it
-- there, on the route the `UPDATE` one could not cover.
-- @setup
CREATE TABLE ocdscr (a INT UNIQUE, b INT);
CREATE TABLE ocdscre (b INT);
INSERT INTO ocdscr VALUES (1, 1);

-- @expect-error: column "zzz" does not exist (42703)
-- @no-profile-lock: the refusal is raised while the statement is lowered, before the arbiter inference runs
-- @test
INSERT INTO ocdscr VALUES (1, 2) ON CONFLICT (a) DO UPDATE SET b = 1, b = 2 RETURNING (SELECT b FROM ocdscre ORDER BY zzz);
