-- A key-moving upsert's `RETURNING` subquery reads the child the `ON UPDATE CASCADE` has not
-- yet repointed: the action runs after `RETURNING` is projected.
-- @profile-lock
-- @setup
CREATE TABLE urbc_p (id INT PRIMARY KEY, v INT);
CREATE TABLE urbc_c (pid INT REFERENCES urbc_p(id) ON UPDATE CASCADE, x INT);
INSERT INTO urbc_p VALUES (1, 1), (2, 2);
INSERT INTO urbc_c VALUES (1, 10), (2, 20);

-- @test
INSERT INTO urbc_p VALUES (1, 5) ON CONFLICT (id) DO UPDATE SET id = 9
RETURNING id, (SELECT pid FROM urbc_c WHERE x = 10) AS r;
