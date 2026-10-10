-- An `ON UPDATE CASCADE` writes the referencing rows when an update assigns the referenced
-- column, so a body reading that table is refused.
-- @setup
CREATE TABLE p (id INT PRIMARY KEY);
CREATE TABLE c (pid INT REFERENCES p (id) ON UPDATE CASCADE);
INSERT INTO p VALUES (1), (2);
INSERT INTO c VALUES (1), (1), (2);
-- @expect-error: pglet refuses a read of a table a data-modifying CTE can write (18.6 answers)
-- @profile-lock
-- @test
WITH RECURSIVE w AS (UPDATE p SET id = 5 WHERE id = 2 RETURNING id) SELECT sum(pid) FROM c;
