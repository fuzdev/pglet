-- The same two-hop reach with `ON DELETE CASCADE` at the second hop: the cascade from `p`
-- deletes `c` rows, whose own deletion cascades into `g` through `c.id`.
-- @setup
CREATE TABLE p (id INT PRIMARY KEY);
CREATE TABLE c (id INT PRIMARY KEY, pid INT REFERENCES p (id) ON DELETE CASCADE);
CREATE TABLE g (cid INT REFERENCES c (id) ON DELETE CASCADE);
INSERT INTO p VALUES (1), (2);
INSERT INTO c VALUES (10, 1), (20, 2);
INSERT INTO g VALUES (10), (20);
-- @expect-error: pglet refuses a read of a table a data-modifying CTE can write (18.6 answers)
-- @profile-lock
-- @test
WITH RECURSIVE d AS (DELETE FROM p WHERE id = 2 RETURNING id) SELECT count(*) FROM g;
