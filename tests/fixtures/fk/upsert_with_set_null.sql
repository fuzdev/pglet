-- The plain `WITH` spelling of a key-moving upsert performs the `SET NULL` on the child.
-- @profile-lock
-- @setup
CREATE TABLE upws_p (id INT PRIMARY KEY, v INT);
CREATE TABLE upws_c (pid INT REFERENCES upws_p(id) ON UPDATE SET NULL, x INT);
INSERT INTO upws_p VALUES (1, 1), (2, 2);
INSERT INTO upws_c VALUES (1, 10), (2, 20);
WITH w AS (INSERT INTO upws_p VALUES (1, 5) ON CONFLICT (id) DO UPDATE SET id = 9 RETURNING id) SELECT * FROM w;

-- @test
SELECT pid, x FROM upws_c ORDER BY x;
