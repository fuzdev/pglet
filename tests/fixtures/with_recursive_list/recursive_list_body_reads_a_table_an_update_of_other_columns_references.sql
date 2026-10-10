-- An `ON UPDATE CASCADE` writes the referencing rows only when an update assigns a
-- referenced column, so an update of another column leaves the referencing table unread.
-- @setup
CREATE TABLE p (id INT PRIMARY KEY, v INT);
CREATE TABLE c (pid INT REFERENCES p (id) ON UPDATE CASCADE ON DELETE CASCADE);
INSERT INTO p VALUES (1, 0), (2, 0);
INSERT INTO c VALUES (1), (1), (2);
-- @profile-lock
-- @test
WITH RECURSIVE w AS (UPDATE p SET v = 7 WHERE id = 1 RETURNING id) SELECT sum(pid) FROM c;
