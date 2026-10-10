-- A `COALESCE` left operand with a computed `float8` branch is `float8`: the list is
-- `float8`, and a `real` `0.1` widened to it equals neither constant.
-- @setup
CREATE TABLE inl (id INT, x REAL, d FLOAT8, n NUMERIC, i INT, b BIGINT, s SMALLINT);
INSERT INTO inl VALUES
  (1, 0.1, 0.1, 0.1, 1, 1, 1),
  (2, 1.5, 1.5, 1.5, 2, 2, 2),
  (3, 16777216, 9007199254740992, 16777217, 16777217, 9007199254740993, 3),
  (4, NULL, NULL, NULL, NULL, NULL, NULL);
-- @test
SELECT id FROM inl WHERE COALESCE(x + 0::float8, 1.5::real) IN (0.1, 0.3) ORDER BY id;
