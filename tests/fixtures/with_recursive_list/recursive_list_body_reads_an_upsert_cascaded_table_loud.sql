-- An `INSERT … ON CONFLICT DO UPDATE` assigning the referenced column is an update of it,
-- so an `ON UPDATE CASCADE` writes the referencing rows and a body reading them is refused.
-- @setup
CREATE TABLE p (id INT PRIMARY KEY);
CREATE TABLE c (pid INT REFERENCES p (id) ON UPDATE CASCADE);
INSERT INTO p VALUES (1), (2);
INSERT INTO c VALUES (1), (1), (2);
-- @expect-error: pglet refuses a read of a table a data-modifying CTE can write (18.6 answers)
-- @profile-lock
-- @test
WITH RECURSIVE w AS (INSERT INTO p VALUES (2) ON CONFLICT (id) DO UPDATE SET id = 5 RETURNING id)
SELECT sum(pid) FROM c;
