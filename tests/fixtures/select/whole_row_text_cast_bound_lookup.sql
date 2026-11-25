-- The row as text, looked up by a bound key: compared before and after to see that a
-- statement left it unchanged.
-- @setup
CREATE TABLE wrs (id TEXT PRIMARY KEY, account INT, expires_at TIMESTAMPTZ);
INSERT INTO wrs VALUES ('s1', 7, '2026-03-04 05:06:07+00'), ('s2', 8, NULL);
-- @profile-lock
-- @param $1 text s1
-- @test
SELECT wrs::text FROM wrs WHERE id = $1;
