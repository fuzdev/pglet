-- An `ON UPDATE CASCADE` converts the parent's `numeric` key to the `int` child column as an
-- assignment does: `2.6` is stored as `3`. The child still references a live key, `3`.
-- Unconverted, the `int` column held `2.6`.
-- @setup
CREATE TABLE p (k numeric PRIMARY KEY);
CREATE TABLE c (id int PRIMARY KEY, pk int REFERENCES p ON UPDATE CASCADE);
INSERT INTO p VALUES (1), (3);
INSERT INTO c VALUES (1, 1);
UPDATE p SET k = 2.6 WHERE k = 1;
-- @test
-- @profile-lock
SELECT id, pk, pk / 3 AS third FROM c;
