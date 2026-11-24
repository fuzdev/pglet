-- A `NOT VALID` check holds an update of a row it never checked: `23514`, even for a change to another column.
-- @profile-lock
-- @setup
CREATE TABLE nv (id int PRIMARY KEY, a int);
INSERT INTO nv VALUES (1, -1), (2, 5);
-- @expect-error: new row for relation "nv" violates check constraint "nv_pos"
-- @test
ALTER TABLE nv ADD CONSTRAINT nv_pos CHECK (a > 0) NOT VALID;
UPDATE nv SET id = 10 WHERE id = 1;
