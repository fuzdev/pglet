-- A `DO UPDATE … WHERE` that skips the conflicting row rewrites nothing, so an
-- `ON UPDATE RESTRICT` child does not refuse it.
-- @profile-lock
-- @setup
CREATE TABLE upwh_p (id INT PRIMARY KEY, v INT);
CREATE TABLE upwh_c (pid INT REFERENCES upwh_p(id) ON UPDATE RESTRICT, x INT);
INSERT INTO upwh_p VALUES (1, 1), (2, 2);
INSERT INTO upwh_c VALUES (1, 10);

-- @test
INSERT INTO upwh_p VALUES (1, 5) ON CONFLICT (id) DO UPDATE SET id = 9 WHERE upwh_p.v = 100 RETURNING id;
