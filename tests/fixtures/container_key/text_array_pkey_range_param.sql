-- A bound range over a `text[]` primary key: at or above `{a,b}` and below `{b}`. The
-- elements are lower-case ASCII, whose order no collation changes.
-- @profile-lock
-- @setup
CREATE TABLE ckr_t (n INT, tags TEXT[] PRIMARY KEY);
INSERT INTO ckr_t (tags, n) VALUES ('{a,b}', 1), ('{ab}', 2), ('{}', 3), ('{a}', 4), ('{b,a}', 5), ('{a,b,c}', 6), ('{b}', 7);
-- @param $1 text[] {a,b}
-- @param $2 text[] {b}
-- @test
SELECT n, tags FROM ckr_t WHERE tags >= $1 AND tags < $2 ORDER BY tags;
