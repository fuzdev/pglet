-- The list is typed the same in a projection, a `CASE` condition, `GROUP BY` and `HAVING`.
-- @setup
CREATE TABLE inl (id INT, x REAL, d FLOAT8, n NUMERIC, i INT, b BIGINT, s SMALLINT);
INSERT INTO inl VALUES
  (1, 0.1, 0.1, 0.1, 1, 1, 1),
  (2, 1.5, 1.5, 1.5, 2, 2, 2),
  (3, 16777216, 9007199254740992, 16777217, 16777217, 9007199254740993, 3),
  (4, NULL, NULL, NULL, NULL, NULL, NULL);
-- @test
SELECT x IN (0.1, 1.5) AS hit, count(*) AS rows, sum(CASE WHEN x IN (0.1, 7) THEN 1 ELSE 0 END) AS tenths
FROM inl GROUP BY x IN (0.1, 1.5) HAVING x IN (0.1, 1.5) IS NOT NULL ORDER BY 1;
