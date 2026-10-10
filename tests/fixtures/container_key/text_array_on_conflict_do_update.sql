-- `ON CONFLICT (tags)` arbitrates on the array key.
-- @profile-lock
-- @setup
CREATE TABLE ckc_t (n INT, tags TEXT[] PRIMARY KEY);
INSERT INTO ckc_t (tags, n) VALUES ('{a,b}', 1);
INSERT INTO ckc_t (tags, n) VALUES ('{a,b}', 5) ON CONFLICT (tags) DO UPDATE SET n = ckc_t.n + excluded.n;
INSERT INTO ckc_t (tags, n) VALUES ('{a}', 7), ('{a,b,c}', 8) ON CONFLICT (tags) DO UPDATE SET n = ckc_t.n + excluded.n;
-- @test
SELECT n, tags FROM ckc_t ORDER BY n;
