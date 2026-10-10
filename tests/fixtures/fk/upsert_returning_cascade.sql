-- The `RETURNING` spelling of a key-moving upsert cascades the key into the child too.
-- @profile-lock
-- @setup
CREATE TABLE uprc_p (id INT PRIMARY KEY, v INT);
CREATE TABLE uprc_c (pid INT REFERENCES uprc_p(id) ON UPDATE CASCADE, x INT);
INSERT INTO uprc_p VALUES (1, 1), (2, 2);
INSERT INTO uprc_c VALUES (1, 10), (2, 20);
INSERT INTO uprc_p VALUES (1, 5) ON CONFLICT (id) DO UPDATE SET id = 9 RETURNING id;

-- @test
SELECT pid, x FROM uprc_c ORDER BY x;
