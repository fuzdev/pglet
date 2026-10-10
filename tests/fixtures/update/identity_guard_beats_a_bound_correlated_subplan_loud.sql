-- The accept-side control for the two above: a correlated sub-plan the re-walk *binds*
-- takes no verdict away, and the guard behind it still answers `428C9`. Without this the
-- pair above would pass on a re-walk that refused everything.
-- @setup
CREATE TABLE igcb (id INT GENERATED ALWAYS AS IDENTITY, b INT);
CREATE TABLE igcbe (b INT);
INSERT INTO igcb (b) VALUES (0);

-- @expect-error: column "id" can only be updated to DEFAULT (428C9)
-- @no-profile-lock: the refusal is raised while the statement is lowered, before any index selection runs
-- @test
UPDATE igcb SET id = 5 RETURNING (SELECT b FROM igcbe);
