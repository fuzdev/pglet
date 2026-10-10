-- A `NO ACTION` foreign key writes nothing when its referenced table is deleted from, so
-- a body reading the referencing table is no read of a write.
-- @setup
CREATE TABLE p (id INT PRIMARY KEY);
CREATE TABLE c (pid INT REFERENCES p (id) ON UPDATE CASCADE);
INSERT INTO p VALUES (1), (2), (3);
INSERT INTO c VALUES (1), (1), (2);
-- @profile-lock
-- @test
WITH RECURSIVE w AS (DELETE FROM p WHERE id = 3 RETURNING id) SELECT count(*) FROM c;
