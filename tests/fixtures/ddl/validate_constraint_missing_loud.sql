-- `VALIDATE CONSTRAINT` of a name the table has no constraint under: `42704`.
-- @profile-lock
-- @setup
CREATE TABLE nv (id int PRIMARY KEY, a int);
INSERT INTO nv VALUES (1, -1), (2, 5);
-- @expect-error: constraint "nosuch" of relation "nv" does not exist
-- @test
ALTER TABLE nv VALIDATE CONSTRAINT nosuch;
