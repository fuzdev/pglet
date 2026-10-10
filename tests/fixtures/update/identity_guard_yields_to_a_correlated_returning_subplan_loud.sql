-- The identity guard shares the duplicate `SET` target's position — it is the same
-- rewriter check, one loop later — so it shares this boundary too: the correlated
-- re-walk's `42703` outranks it. Above the re-walk the guard answered `428C9`.
-- @setup
CREATE TABLE igcr (id INT GENERATED ALWAYS AS IDENTITY, b INT);
CREATE TABLE igcre (b INT);
INSERT INTO igcr (b) VALUES (0);

-- @expect-error: column "zzz" does not exist (42703)
-- @no-profile-lock: the refusal is raised while the statement is lowered, before any index selection runs
-- @test
UPDATE igcr SET id = 5 RETURNING (SELECT b FROM igcre ORDER BY zzz);
