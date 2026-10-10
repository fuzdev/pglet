-- A derived table's column out of a `COALESCE` with a computed `float8` branch is
-- `float8`, and so is the list over it: the `real` `0.1` widened is not among its items.
-- @setup
CREATE TABLE inl (id INT, x REAL, d FLOAT8, n NUMERIC, i INT, b BIGINT, s SMALLINT);
INSERT INTO inl VALUES
  (1, 0.1, 0.1, 0.1, 1, 1, 1),
  (2, 1.5, 1.5, 1.5, 2, 2, 2),
  (3, 16777216, 9007199254740992, 16777217, 16777217, 9007199254740993, 3),
  (4, NULL, NULL, NULL, NULL, NULL, NULL);
-- @test
SELECT id FROM (SELECT id, COALESCE(x + 0::float8, 1.5::real) AS y FROM inl) s WHERE y NOT IN (0.1, 1.5) ORDER BY id;
