-- A left operand multiplying a `real` by the enclosing query's `float8` column is
-- `float8`: the list is `float8`, and the `float8` `0.1` and `1.5` are among its items.
-- @setup
CREATE TABLE inl (id INT, x REAL, d FLOAT8, n NUMERIC, i INT, b BIGINT, s SMALLINT);
INSERT INTO inl VALUES
  (1, 0.1, 0.1, 0.1, 1, 1, 1),
  (2, 1.5, 1.5, 1.5, 2, 2, 2),
  (3, 16777216, 9007199254740992, 16777217, 16777217, 9007199254740993, 3),
  (4, NULL, NULL, NULL, NULL, NULL, NULL);
-- @test
SELECT o.id FROM inl o WHERE EXISTS (SELECT 1 FROM inl q WHERE q.id = 2 AND (q.x - 0.5::real) * o.d IN (0.1, 1.5)) ORDER BY o.id;
