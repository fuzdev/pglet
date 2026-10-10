-- The control the refusal must not reach: both proposals meet the row, and the `WHERE` is
-- false for both, so neither writes and 18.6 answers `INSERT 0 0` with no error.
-- @profile-lock
-- @setup
CREATE TABLE ocwn (id INT PRIMARY KEY, a INT);
INSERT INTO ocwn VALUES (1, 10);

-- @test
INSERT INTO ocwn VALUES (1, 1), (1, 2)
ON CONFLICT (id) DO UPDATE SET a = excluded.a WHERE ocwn.a < 0;
