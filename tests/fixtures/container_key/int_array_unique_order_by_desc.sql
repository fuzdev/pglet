-- The descending walk of `int_array_unique_order_by.sql`'s key, under a `LIMIT`.
-- @profile-lock
-- @setup
CREATE TABLE ckd_a (id INT, a INT[] NOT NULL UNIQUE);
INSERT INTO ckd_a (a, id) VALUES
  ('{2}', 7), ('{NULL,1}', 9), ('{1,2}', 4), ('{}', 1), ('{NULL,NULL}', 10), ('{-1,5,5}', 2),
  ('{1,NULL}', 6), ('{1}', 3), ('{NULL}', 8), ('{1,2,3}', 5);
-- @test
SELECT id, a FROM ckd_a ORDER BY a DESC LIMIT 4;
