-- A range over an `int[]` unique key, both bounds bound: above `{1,2}` and below
-- `{NULL}` lie its extension `{1,2,3}`, the NULL-element `{1,NULL}`, and `{2}`.
-- @profile-lock
-- @setup
CREATE TABLE ckr_a (id INT, a INT[] NOT NULL UNIQUE);
INSERT INTO ckr_a (a, id) VALUES
  ('{2}', 7), ('{NULL,1}', 9), ('{1,2}', 4), ('{}', 1), ('{NULL,NULL}', 10), ('{-1,5,5}', 2),
  ('{1,NULL}', 6), ('{1}', 3), ('{NULL}', 8), ('{1,2,3}', 5);
-- @param $1 int4[] {1,2}
-- @param $2 int4[] {NULL}
-- @test
SELECT id, a FROM ckr_a WHERE a > $1 AND a < $2 ORDER BY a;
