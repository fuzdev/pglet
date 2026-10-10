-- A `real` item makes the list `real`: the integer constant beside it is rounded to
-- `16777216`, which the stored `16777217` does not equal at `float8`.
-- @setup
CREATE TABLE inl (id INT, x REAL, d FLOAT8, n NUMERIC, i INT, b BIGINT, s SMALLINT);
INSERT INTO inl VALUES
  (1, 0.1, 0.1, 0.1, 1, 1, 1),
  (2, 1.5, 1.5, 1.5, 2, 2, 2),
  (3, 16777216, 9007199254740992, 16777217, 16777217, 9007199254740993, 3),
  (4, NULL, NULL, NULL, NULL, NULL, NULL);
-- @test
SELECT id, i IN (16777217, 2.5::real) AS hit, i IN (16777216, 2.5::real) AS near FROM inl ORDER BY id;
