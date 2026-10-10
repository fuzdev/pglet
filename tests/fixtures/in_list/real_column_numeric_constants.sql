-- An `IN` list of two or more items naming no column is one array of the common type of the
-- left operand and those items: beside a `real` column the numeric constants are `real`, so
-- `0.1` matches the stored `0.1`.
-- @setup
CREATE TABLE inl (id INT, x REAL, d FLOAT8, n NUMERIC, i INT, b BIGINT, s SMALLINT);
INSERT INTO inl VALUES
  (1, 0.1, 0.1, 0.1, 1, 1, 1),
  (2, 1.5, 1.5, 1.5, 2, 2, 2),
  (3, 16777216, 9007199254740992, 16777217, 16777217, 9007199254740993, 3),
  (4, NULL, NULL, NULL, NULL, NULL, NULL);
-- @test
SELECT id FROM inl WHERE x IN (0.1, 1.5) ORDER BY id;
