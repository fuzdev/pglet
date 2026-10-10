-- The orphan-cleanup idiom with its sub-select written in a second pair of parentheses:
-- still the `IN` sub-select form, so over an empty parent every row goes, not none.
-- @setup
CREATE TABLE parent (id INT);
CREATE TABLE child (x INT UNIQUE);
INSERT INTO child VALUES (1), (NULL), (3);
-- @profile-lock
-- @test
DELETE FROM child WHERE x NOT IN ((SELECT id FROM parent));
SELECT count(*) FROM child;
