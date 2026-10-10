-- An `INSERT` into a referenced table writes nothing through the foreign key, so a body
-- reading the referencing table is no read of a write.
-- @setup
CREATE TABLE p (id INT PRIMARY KEY);
CREATE TABLE c (pid INT REFERENCES p (id) ON DELETE CASCADE ON UPDATE CASCADE);
INSERT INTO p VALUES (1), (2);
INSERT INTO c VALUES (1), (1), (2);
-- @test
WITH RECURSIVE w AS (INSERT INTO p VALUES (3) RETURNING id) SELECT count(*) FROM c;
