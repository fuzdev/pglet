-- A cascade repoints a child to a referenced key moved to NULL, and the child's NOT NULL
-- column refuses it.
-- @setup
CREATE TABLE p (k text UNIQUE);
CREATE TABLE c (id int, k text NOT NULL REFERENCES p (k) ON UPDATE CASCADE);
INSERT INTO p VALUES ('a');
INSERT INTO c VALUES (1, 'a');
-- @test
-- @expect-error: the repointed NULL in a NOT NULL column
-- @expect-message
-- @profile-lock
UPDATE p SET k = NULL;
