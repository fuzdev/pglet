-- Without `NOT VALID` the same check reads the rows: `23514 … is violated by some row`.
-- @profile-lock
-- @setup
CREATE TABLE nv (id int PRIMARY KEY, a int);
INSERT INTO nv VALUES (1, -1), (2, 5);
-- @expect-error: check constraint "nv_pos" of relation "nv" is violated by some row
-- @test
ALTER TABLE nv ADD CONSTRAINT nv_pos CHECK (a > 0);
