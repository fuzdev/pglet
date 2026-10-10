-- The `NOT IN` of a list a `float8` parameter widened: the stored `real` `0.1` is not
-- among its `float8` items, so its row is kept.
-- @param $1 float8 5
-- @setup
CREATE TABLE inl (id INT, x REAL, d FLOAT8, n NUMERIC, i INT, b BIGINT, s SMALLINT);
INSERT INTO inl VALUES
  (1, 0.1, 0.1, 0.1, 1, 1, 1),
  (2, 1.5, 1.5, 1.5, 2, 2, 2),
  (3, 16777216, 9007199254740992, 16777217, 16777217, 9007199254740993, 3),
  (4, NULL, NULL, NULL, NULL, NULL, NULL);
-- @test
SELECT id FROM inl WHERE d < $1 AND x NOT IN ($1, 0.1) ORDER BY id;
