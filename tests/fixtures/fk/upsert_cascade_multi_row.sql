-- Each row a multi-row `ON CONFLICT DO UPDATE` rewrites cascades its own key.
-- @profile-lock
-- @setup
CREATE TABLE upcm_p (id INT PRIMARY KEY);
CREATE TABLE upcm_c (pid INT REFERENCES upcm_p(id) ON UPDATE CASCADE, x INT);
INSERT INTO upcm_p VALUES (1), (2), (3);
INSERT INTO upcm_c VALUES (1, 10), (2, 20), (3, 30);
INSERT INTO upcm_p VALUES (1), (2) ON CONFLICT (id) DO UPDATE SET id = upcm_p.id + 10;

-- @test
SELECT pid, x FROM upcm_c ORDER BY x;
