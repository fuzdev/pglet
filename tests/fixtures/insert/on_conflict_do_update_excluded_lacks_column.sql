-- The `excluded` pseudo-relation is a frame that owns its qualifier, so a column it lacks
-- is `42703 column excluded.nosuch does not exist` on 18.6 — the column half of the
-- binding family. pglet answered `42P01 "excluded"`, naming a relation that is very much
-- in scope.
--
-- Its neighbours are the two `42P01` fixtures beside it (a qualifier the action's
-- namespace does not hold), and together the three say the split follows Postgres's:
-- whether a frame owns the qualifier decides which error, not which clause it sits in.
-- @setup
CREATE TABLE oce_t (id INT PRIMARY KEY, v TEXT);
INSERT INTO oce_t VALUES (1, 'orig');

-- @expect-error: column excluded.nosuch does not exist (42703)
-- @no-profile-lock: the arbiter's unique-index probe is constraint enforcement rather than acceleration and runs identically on both profiles; the refusal is raised evaluating the DO UPDATE action, which selects no index
-- @test
INSERT INTO oce_t VALUES (1, 'new') ON CONFLICT (id) DO UPDATE SET v = EXCLUDED.nosuch;
