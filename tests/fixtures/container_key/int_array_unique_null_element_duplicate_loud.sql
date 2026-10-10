-- Two arrays with a NULL element in the same position are one key: the unique index
-- refuses the repeat, where a NULL **key** would repeat freely.
-- @profile-lock
-- @setup
CREATE TABLE cku_a (id INT, a INT[] NOT NULL UNIQUE);
INSERT INTO cku_a (a, id) VALUES
  ('{2}', 7), ('{NULL,1}', 9), ('{1,2}', 4), ('{}', 1), ('{NULL,NULL}', 10), ('{-1,5,5}', 2),
  ('{1,NULL}', 6), ('{1}', 3), ('{NULL}', 8), ('{1,2,3}', 5);
-- @test
-- @expect-error: a duplicate array key with a NULL element (23505)
-- @expect-message
INSERT INTO cku_a (a, id) VALUES ('{1,NULL}', 11);
