-- A key-moving upsert's `RETURNING` subquery reads the child the `ON UPDATE SET NULL` has not
-- yet nulled.
-- @profile-lock
-- @setup
CREATE TABLE urbn_p (id INT PRIMARY KEY, v INT);
CREATE TABLE urbn_c (pid INT REFERENCES urbn_p(id) ON UPDATE SET NULL, x INT);
INSERT INTO urbn_p VALUES (1, 1), (2, 2);
INSERT INTO urbn_c VALUES (1, 10), (2, 20);

-- @test
INSERT INTO urbn_p VALUES (1, 5) ON CONFLICT (id) DO UPDATE SET id = 9
RETURNING id, (SELECT pid FROM urbn_c WHERE x = 10) AS r;
