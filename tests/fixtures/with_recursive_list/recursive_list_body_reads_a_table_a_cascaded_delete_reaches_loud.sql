-- A row an `ON DELETE CASCADE` removes is a delete of its table, so the foreign keys
-- referencing any of that table's columns act on it — here `g`'s `ON DELETE SET NULL` over
-- `c.id`, a column the cascade's own foreign key does not name — and a body reading `g`
-- is refused.
-- @setup
CREATE TABLE p (id INT PRIMARY KEY);
CREATE TABLE c (id INT PRIMARY KEY, pid INT REFERENCES p (id) ON DELETE CASCADE);
CREATE TABLE g (cid INT REFERENCES c (id) ON DELETE SET NULL);
INSERT INTO p VALUES (1), (2);
INSERT INTO c VALUES (10, 1), (20, 2);
INSERT INTO g VALUES (10), (20);
-- @expect-error: pglet refuses a read of a table a data-modifying CTE can write (18.6 answers)
-- @profile-lock
-- @test
WITH RECURSIVE d AS (DELETE FROM p WHERE id = 2 RETURNING id) SELECT count(cid) FROM g;
