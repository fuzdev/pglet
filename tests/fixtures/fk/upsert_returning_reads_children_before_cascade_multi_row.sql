-- Each row of a multi-row key-moving upsert projects its `RETURNING` subquery over the
-- children as they stood before any `ON UPDATE CASCADE` ran.
-- @profile-lock
-- @setup
CREATE TABLE urbm_p (id INT PRIMARY KEY, v INT);
CREATE TABLE urbm_c (pid INT REFERENCES urbm_p(id) ON UPDATE CASCADE, x INT);
INSERT INTO urbm_p VALUES (1, 1), (2, 2);
INSERT INTO urbm_c VALUES (1, 10), (2, 20);

-- @test
INSERT INTO urbm_p VALUES (1, 5), (2, 6) ON CONFLICT (id) DO UPDATE SET id = urbm_p.id + 10
RETURNING id, (SELECT sum(pid) FROM urbm_c) AS s;
