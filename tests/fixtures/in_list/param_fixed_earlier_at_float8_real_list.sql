-- A parameter an earlier comparison fixed at `float8` is a `float8` item of the list, so
-- the list is `float8` and the constant `0.1` does not match the stored `real`.
-- @param $1 float8 5
-- @setup
CREATE TABLE inl (id INT, x REAL, d FLOAT8, n NUMERIC, i INT, b BIGINT, s SMALLINT);
INSERT INTO inl VALUES
  (1, 0.1, 0.1, 0.1, 1, 1, 1),
  (2, 1.5, 1.5, 1.5, 2, 2, 2),
  (3, 16777216, 9007199254740992, 16777217, 16777217, 9007199254740993, 3),
  (4, NULL, NULL, NULL, NULL, NULL, NULL);
-- @test
SELECT id FROM inl WHERE d < $1 AND x IN ($1, 0.1) ORDER BY id;
