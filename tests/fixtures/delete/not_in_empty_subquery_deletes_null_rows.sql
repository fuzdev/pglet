-- The orphan-cleanup idiom over an empty parent: `x NOT IN (<empty>)` is `true` for every
-- row, the NULL one included, so both rows go.
-- @setup
CREATE TABLE parent (id INT);
CREATE TABLE child (x INT UNIQUE);
INSERT INTO child VALUES (1), (NULL);
-- @profile-lock
-- @test
DELETE FROM child WHERE x NOT IN (SELECT id FROM parent);
SELECT count(*) FROM child;
