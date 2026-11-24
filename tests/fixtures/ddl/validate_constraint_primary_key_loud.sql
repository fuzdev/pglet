-- `VALIDATE CONSTRAINT` of a primary key: `42809`, with 18.7's detail.
-- @profile-lock
-- @setup
CREATE TABLE nv (id int PRIMARY KEY, a int);
INSERT INTO nv VALUES (1, -1), (2, 5);
-- @expect-error: cannot validate constraint "nv_pkey" of relation "nv"
-- @test
ALTER TABLE nv VALIDATE CONSTRAINT nv_pkey;
