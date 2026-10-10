-- The empty array is a key like any other: one row may hold it.
-- @profile-lock
-- @setup
CREATE TABLE ckz_a (id INT, a INT[] NOT NULL UNIQUE);
INSERT INTO ckz_a (a, id) VALUES
  ('{2}', 7), ('{NULL,1}', 9), ('{1,2}', 4), ('{}', 1), ('{NULL,NULL}', 10), ('{-1,5,5}', 2),
  ('{1,NULL}', 6), ('{1}', 3), ('{NULL}', 8), ('{1,2,3}', 5);
-- @test
-- @expect-error: a duplicate empty array key (23505)
-- @expect-message
INSERT INTO ckz_a (a, id) VALUES ('{}', 11);
