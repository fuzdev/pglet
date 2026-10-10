-- The `GENERATED ALWAYS` guard is a **rewriter** check on 18.6, so it runs after every
-- parse-analysis error in the action — including one in the `WHERE`, which is the last of
-- the three parse-analysis passes. `42703`, not `428C9`.
--
-- It is the fourth position in the action's order and the only one that could not be read
-- off the other three: the guard concerns the same column an assignment target does, so
-- nothing about passes 1–3 says whether it sits inside pass 2 or after pass 3. Its accept
-- polarity — `SET n = 5` alone raising `428C9` — is
-- `insert/on_conflict_action_identity_always.sql`.
-- @setup
CREATE TABLE ocig (n INT GENERATED ALWAYS AS IDENTITY PRIMARY KEY, v TEXT);
INSERT INTO ocig (v) VALUES ('a');

-- @expect-error: column "nosuchcol" does not exist (42703)
-- @no-profile-lock: the refusal is raised while the ON CONFLICT action is lowered, before any index selection runs
-- @test
INSERT INTO ocig (v) VALUES ('b') ON CONFLICT (n) DO UPDATE SET n = 5 WHERE nosuchcol = 1;
