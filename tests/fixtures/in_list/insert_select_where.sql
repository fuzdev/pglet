-- `INSERT … SELECT` filters and projects through the list.
-- @setup
CREATE TABLE inl (id INT, x REAL, d FLOAT8, n NUMERIC, i INT, b BIGINT, s SMALLINT);
INSERT INTO inl VALUES
  (1, 0.1, 0.1, 0.1, 1, 1, 1),
  (2, 1.5, 1.5, 1.5, 2, 2, 2),
  (3, 16777216, 9007199254740992, 16777217, 16777217, 9007199254740993, 3),
  (4, NULL, NULL, NULL, NULL, NULL, NULL);
-- @test
INSERT INTO inl (id, x, i) SELECT id + 10, x, CASE WHEN x IN (0.1, 1.5) THEN 1 ELSE 0 END FROM inl WHERE x IN (0.1, 16777217) RETURNING id, x, i;
