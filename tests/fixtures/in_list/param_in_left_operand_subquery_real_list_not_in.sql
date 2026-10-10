-- A parameter inside a subquery of the left operand, fixed at `float8` by an earlier
-- comparison: the left operand is `float8`, and so is the list.
-- @param $1 float8 1
-- @setup
CREATE TABLE inl (id INT, x REAL, d FLOAT8, n NUMERIC, i INT, b BIGINT, s SMALLINT);
INSERT INTO inl VALUES
  (1, 0.1, 0.1, 0.1, 1, 1, 1),
  (2, 1.5, 1.5, 1.5, 2, 2, 2),
  (3, 16777216, 9007199254740992, 16777217, 16777217, 9007199254740993, 3),
  (4, NULL, NULL, NULL, NULL, NULL, NULL);
-- @test
SELECT id FROM inl WHERE d < $1 AND x * (SELECT $1) NOT IN (0.1, 1.5) ORDER BY id;
