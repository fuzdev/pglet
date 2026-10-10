-- `ON CONFLICT (j)` arbitrates on the `jsonb` key: `[1.0, 2]` conflicts with the stored
-- `[1, 2]` and takes the `DO UPDATE` arm, `[1, 3]` inserts.
-- @profile-lock
-- @setup
CREATE TABLE ckc_j (n INT, j JSONB PRIMARY KEY);
INSERT INTO ckc_j (j, n) VALUES ('[1,2]', 1);
INSERT INTO ckc_j (j, n) VALUES ('[1.0,2]', 5) ON CONFLICT (j) DO UPDATE SET n = ckc_j.n + excluded.n;
INSERT INTO ckc_j (j, n) VALUES ('[1,3]', 7) ON CONFLICT (j) DO UPDATE SET n = ckc_j.n + excluded.n;
-- @test
SELECT n, j::text AS key FROM ckc_j ORDER BY n;
