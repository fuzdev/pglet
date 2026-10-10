-- A `float8` item makes the list `float8`, and the numeric constant beside it is rounded to
-- it: the stored `0.1` matches a constant that differs past `float8`'s precision.
-- @setup
CREATE TABLE inl (id INT, x REAL, d FLOAT8, n NUMERIC, i INT, b BIGINT, s SMALLINT);
INSERT INTO inl VALUES
  (1, 0.1, 0.1, 0.1, 1, 1, 1),
  (2, 1.5, 1.5, 1.5, 2, 2, 2),
  (3, 16777216, 9007199254740992, 16777217, 16777217, 9007199254740993, 3),
  (4, NULL, NULL, NULL, NULL, NULL, NULL);
-- @test
SELECT id FROM inl WHERE n IN (0.1000000000000000000001, 7.5::float8) ORDER BY id;
