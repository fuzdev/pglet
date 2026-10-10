-- An `ON DELETE SET NULL` is an update of the referencing table's key columns, so a foreign
-- key on those columns carries it on: its `ON UPDATE CASCADE` writes a further table, and a
-- body reading that one is refused.
-- @setup
CREATE TABLE p (id INT PRIMARY KEY);
CREATE TABLE c (pid INT UNIQUE REFERENCES p (id) ON DELETE SET NULL);
CREATE TABLE g (cpid INT REFERENCES c (pid) ON UPDATE CASCADE);
INSERT INTO p VALUES (1), (2);
INSERT INTO c VALUES (1), (2);
INSERT INTO g VALUES (1), (2);
-- @expect-error: pglet refuses a read of a table a data-modifying CTE can write (18.6 answers)
-- @profile-lock
-- @test
WITH RECURSIVE w AS (DELETE FROM p WHERE id = 2 RETURNING id) SELECT count(cpid) FROM g;
