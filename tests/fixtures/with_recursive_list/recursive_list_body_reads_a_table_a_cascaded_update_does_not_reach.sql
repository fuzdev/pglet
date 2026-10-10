-- A cascaded update writes the referencing table's foreign-key columns, which reaches a
-- further table only through a foreign key on those columns: one on the table's primary key
-- is untouched, whatever its `ON DELETE`.
-- @setup
CREATE TABLE p (id INT PRIMARY KEY);
CREATE TABLE c (id INT PRIMARY KEY, pid INT REFERENCES p (id) ON UPDATE CASCADE);
CREATE TABLE g (cid INT REFERENCES c (id) ON DELETE CASCADE);
INSERT INTO p VALUES (1), (2);
INSERT INTO c VALUES (10, 1), (20, 2);
INSERT INTO g VALUES (10), (20);
-- @profile-lock
-- @test
WITH RECURSIVE w AS (UPDATE p SET id = 5 WHERE id = 2 RETURNING id) SELECT sum(cid) FROM g;
