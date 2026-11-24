-- A `NOT VALID` check still holds every write after it: `23514` for a new row.
-- @profile-lock
-- @setup
CREATE TABLE nv (id int PRIMARY KEY, a int);
INSERT INTO nv VALUES (1, -1), (2, 5);
-- @expect-error: new row for relation "nv" violates check constraint "nv_pos"
-- @test
ALTER TABLE nv ADD CONSTRAINT nv_pos CHECK (a > 0) NOT VALID;
INSERT INTO nv VALUES (3, -3);
