-- A string literal takes the list's common type, not the column's: beside `2.5` it is
-- `numeric`, so `'1.0'` is read and matches and `'1.5'` is no error.
-- @setup
CREATE TABLE inl (id INT, x REAL, d FLOAT8, n NUMERIC, i INT, b BIGINT, s SMALLINT);
INSERT INTO inl VALUES
  (1, 0.1, 0.1, 0.1, 1, 1, 1),
  (2, 1.5, 1.5, 1.5, 2, 2, 2),
  (3, 16777216, 9007199254740992, 16777217, 16777217, 9007199254740993, 3),
  (4, NULL, NULL, NULL, NULL, NULL, NULL);
-- @test
SELECT id, i IN ('1.0', 2.5) AS hit, i IN ('1.5', 2.5) AS miss FROM inl ORDER BY id;
