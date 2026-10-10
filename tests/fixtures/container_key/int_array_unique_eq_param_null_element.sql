-- A bound equality probe whose array holds a NULL element: `array_eq` calls two NULL
-- elements equal, so `{1,NULL}` finds its row.
-- @profile-lock
-- @setup
CREATE TABLE cke_a (id INT, a INT[] NOT NULL UNIQUE);
INSERT INTO cke_a (a, id) VALUES
  ('{2}', 7), ('{NULL,1}', 9), ('{1,2}', 4), ('{}', 1), ('{NULL,NULL}', 10), ('{-1,5,5}', 2),
  ('{1,NULL}', 6), ('{1}', 3), ('{NULL}', 8), ('{1,2,3}', 5);
-- @param $1 int4[] {1,NULL}
-- @test
SELECT id, a FROM cke_a WHERE a = $1;
