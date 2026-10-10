-- `ON CONFLICT (j) DO NOTHING` skips the row whose `jsonb` key is already stored under
-- another scale, and reports the one row it inserted.
-- @profile-lock
-- @setup
CREATE TABLE ckg_j (n INT, j JSONB UNIQUE);
INSERT INTO ckg_j (j, n) VALUES ('{"k":1}', 1);
-- @test
INSERT INTO ckg_j (j, n) VALUES ('{"k":1.0}', 2), ('{"k":2}', 3) ON CONFLICT (j) DO NOTHING RETURNING n;
