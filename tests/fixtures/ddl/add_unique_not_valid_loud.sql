-- `UNIQUE … NOT VALID` is `0A000`, as on 18.7.
-- @profile-lock
-- @setup
CREATE TABLE nv (id int PRIMARY KEY, a int);
INSERT INTO nv VALUES (1, -1), (2, 5);
-- @expect-error: UNIQUE constraints cannot be marked NOT VALID
-- @test
ALTER TABLE nv ADD CONSTRAINT nv_u UNIQUE (a) NOT VALID;
