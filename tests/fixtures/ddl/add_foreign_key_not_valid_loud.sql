-- `FOREIGN KEY … NOT VALID` is refused `0A000`: pglet records no foreign key unvalidated. 18.7 accepts, so any error passes.
-- @profile-lock
-- @setup
CREATE TABLE nv (id int PRIMARY KEY, a int);
INSERT INTO nv VALUES (1, -1), (2, 5);
-- @expect-error: FOREIGN KEY … NOT VALID not supported
-- @test
ALTER TABLE nv ADD CONSTRAINT nv_fk FOREIGN KEY (a) REFERENCES nv (id) NOT VALID;
