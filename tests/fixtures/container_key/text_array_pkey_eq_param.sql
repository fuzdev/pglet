-- A bound equality probe over a `text[]` primary key.
-- @profile-lock
-- @setup
CREATE TABLE cke_t (n INT, tags TEXT[] PRIMARY KEY);
INSERT INTO cke_t (tags, n) VALUES ('{a,b}', 1), ('{ab}', 2), ('{}', 3), ('{a}', 4), ('{b,a}', 5);
-- @param $1 text[] {a,b}
-- @test
SELECT n, tags FROM cke_t WHERE tags = $1;
