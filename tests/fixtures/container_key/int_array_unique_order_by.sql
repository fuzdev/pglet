-- An `int[]` unique key orders as 18.6's `array_ops` does, whichever of an index-ordered
-- scan and a sort answers: elements before length (`{1,2,3}` below `{2}`), a shorter
-- array below one it is a prefix of, a NULL element above every value. The rows are
-- inserted out of order and the key is the table's second column.
-- @profile-lock
-- @setup
CREATE TABLE cko_a (id INT, a INT[] NOT NULL UNIQUE);
INSERT INTO cko_a (a, id) VALUES
  ('{2}', 7), ('{NULL,1}', 9), ('{1,2}', 4), ('{}', 1), ('{NULL,NULL}', 10), ('{-1,5,5}', 2),
  ('{1,NULL}', 6), ('{1}', 3), ('{NULL}', 8), ('{1,2,3}', 5);
-- @test
SELECT id, a FROM cko_a ORDER BY a;
